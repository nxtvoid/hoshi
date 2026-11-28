//! A file shown in the side panel as a table: a CSV or TSV, with its source a click away, or
//! a workbook one sheet at a time. Only the rows and columns in view are built.

use super::panel_message;
use super::sheet_grid::{SheetGrid, one_line};
use crate::m;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::*;
use hoshi_engine::table::{self, Format, Grid, MAX_TEXT_BYTES, TableFailure, TableFile};
use hoshi_ui::prelude::*;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

/// Pixels a character of the source takes, near enough to size its gutter before drawing.
const CHAR_WIDTH: f32 = 7.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Table,
    Source,
}

/// One sheet or text table, ready to draw.
struct Sheet {
    grid: Arc<Grid>,
    view: Entity<SheetGrid>,
}

enum Body {
    Loading,
    Failed(TableFailure),
    Ready {
        format: Format,
        /// The workbook sheet picked; always 0 for text.
        picked: usize,
        /// `None` while the picked sheet is read, or when it couldn't be.
        sheet: Option<Sheet>,
        sheet_failure: Option<TableFailure>,
    },
}

pub struct FileSurface {
    path: String,
    body: Body,
    view: View,
    source: UniformListScrollHandle,
    /// When the file read was last changed.
    modified: Option<SystemTime>,
    reading: Option<Task<()>>,
    indexing: Option<Task<()>>,
}

fn modified(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

fn failure_message(failure: &TableFailure) -> String {
    match failure {
        TableFailure::TooLarge => m::file_too_large(),
        TableFailure::Read(_) => m::file_read_failed(),
    }
}

/// The letters a spreadsheet names column `index` with: A to Z, then AA, AB…
pub fn column_letters(index: usize) -> String {
    let mut letters = Vec::new();
    let mut rest = index + 1;
    while rest > 0 {
        let digit = (rest - 1) % 26;
        letters.push(char::from(b'A' + digit as u8));
        rest = (rest - 1) / 26;
    }
    letters.iter().rev().collect()
}

impl FileSurface {
    pub fn new(path: String, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            path,
            body: Body::Loading,
            view: View::Table,
            source: UniformListScrollHandle::new(),
            modified: None,
            reading: None,
            indexing: None,
        };
        this.read(None, cx);
        this
    }

    /// Reads the file again when it changed since it was read.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.reading.is_some() {
            return;
        }
        let path = self.path.clone();
        self.reading = Some(cx.spawn(async move |this, cx| {
            let now = cx.background_spawn(async move { modified(&path) }).await;
            this.update(cx, |this, cx| {
                this.reading = None;
                if now != this.modified {
                    let sheet = this.picked_sheet();
                    this.read(sheet, cx);
                }
            })
            .ok();
        }));
    }

    /// The name of the workbook sheet picked.
    fn picked_sheet(&self) -> Option<String> {
        match &self.body {
            Body::Ready {
                format: Format::Workbook { sheets },
                picked,
                ..
            } => sheets.get(*picked).cloned(),
            _ => None,
        }
    }

    /// Reads the file, at sheet `keep` when it is a workbook that still has it.
    fn read(&mut self, keep: Option<String>, cx: &mut Context<Self>) {
        let path = self.path.clone();
        self.reading = Some(cx.spawn(async move |this, cx| {
            let (opened, modified) = cx
                .background_spawn(async move {
                    let modified = modified(&path);
                    let mut opened = table::open(Path::new(&path));
                    let kept = match (&opened, keep) {
                        (
                            Ok(TableFile {
                                format: Format::Workbook { sheets },
                                ..
                            }),
                            Some(keep),
                        ) => sheets
                            .iter()
                            .position(|name| *name == keep)
                            .filter(|at| *at > 0),
                        _ => None,
                    };
                    let mut picked = 0;
                    if let (Some(at), Ok(file)) = (kept, &mut opened)
                        && let Format::Workbook { sheets } = &file.format
                        && let Ok(grid) = table::open_sheet(Path::new(&path), &sheets[at])
                    {
                        file.grid = grid;
                        picked = at;
                    }
                    (opened.map(|file| (file, picked)), modified)
                })
                .await;
            this.update(cx, |this, cx| {
                this.reading = None;
                this.modified = modified;
                this.opened(opened, cx);
            })
            .ok();
        }));
    }

    fn opened(&mut self, opened: Result<(TableFile, usize), TableFailure>, cx: &mut Context<Self>) {
        self.body = match opened {
            Ok((file, picked)) => Body::Ready {
                format: file.format,
                picked,
                sheet: Some(sheet(file.grid, cx)),
                sheet_failure: None,
            },
            Err(failure) => {
                eprintln!("hoshi: could not read {}: {failure:?}", self.path);
                Body::Failed(failure)
            }
        };
        self.indexing = None;
        cx.notify();
    }

    fn pick_sheet(&mut self, index: usize, cx: &mut Context<Self>) {
        let Body::Ready {
            format: Format::Workbook { sheets },
            picked,
            sheet,
            sheet_failure,
        } = &mut self.body
        else {
            return;
        };
        let Some(name) = sheets.get(index).cloned() else {
            return;
        };
        if *picked == index {
            return;
        }
        *picked = index;
        *sheet = None;
        *sheet_failure = None;
        cx.notify();
        let path = self.path.clone();
        self.reading = Some(cx.spawn(async move |this, cx| {
            let read = cx
                .background_spawn(async move { table::open_sheet(Path::new(&path), &name) })
                .await;
            this.update(cx, |this, cx| {
                this.reading = None;
                if let Body::Ready {
                    picked,
                    sheet,
                    sheet_failure,
                    ..
                } = &mut this.body
                    && *picked == index
                {
                    match read {
                        Ok(grid) => *sheet = Some(self::sheet(grid, cx)),
                        Err(failure) => {
                            eprintln!("hoshi: could not read {}: {failure:?}", this.path);
                            *sheet_failure = Some(failure);
                        }
                    }
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The rows of the table shown, and how many of them were split into cells.
    #[cfg(test)]
    pub fn shown_rows(&self, cx: &App) -> Option<(usize, usize)> {
        let Body::Ready {
            sheet: Some(sheet), ..
        } = &self.body
        else {
            return None;
        };
        let built = sheet.view.read(cx).built_rows();
        Some((sheet.grid.rows(), built))
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        if self.view == view {
            return;
        }
        self.view = view;
        if let Body::Ready {
            sheet: Some(sheet), ..
        } = &self.body
            && view == View::Source
            && !sheet.grid.lines_indexed()
        {
            let grid = sheet.grid.clone();
            self.indexing = Some(cx.spawn(async move |this, cx| {
                cx.background_spawn(async move { grid.lines() }).await;
                this.update(cx, |this, cx| {
                    this.indexing = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        cx.notify();
    }

    fn toolbar(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let delimited = matches!(
            &self.body,
            Body::Ready {
                format: Format::Delimited { .. },
                ..
            }
        );
        let view_button = |view: View, label: String, cx: &mut Context<Self>| {
            Button::new(match view {
                View::Table => "file-view-table",
                View::Source => "file-view-source",
            })
            .label(label)
            .size(ButtonSize::Xs)
            .variant(if self.view == view {
                ButtonVariant::Outline
            } else {
                ButtonVariant::Ghost
            })
            .on_click(cx.listener(move |this, _, _, cx| this.set_view(view, cx)))
        };
        let views = if delimited {
            vec![
                view_button(View::Table, m::file_view_table(), cx).into_any_element(),
                view_button(View::Source, m::file_view_source(), cx).into_any_element(),
            ]
        } else {
            Vec::new()
        };
        path_bar(&self.path, colors, views)
    }

    fn source_view(&self, grid: &Arc<Grid>, cx: &App) -> AnyElement {
        let colors = cx.hoshi().clone();
        if !grid.lines_indexed() {
            return loading(&colors);
        }
        let mono = cx.theme().mono_font_family.clone();
        let digits = grid.lines().max(1).to_string().len();
        let gutter = px(digits as f32 * CHAR_WIDTH + 16.);
        let lines = grid.clone();
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(
                uniform_list("file-source", grid.lines(), move |range, _, _| {
                    range
                        .map(|index| {
                            div()
                                .flex()
                                .h(px(20.))
                                .items_center()
                                .text_xs()
                                .font_family(mono.clone())
                                .child(
                                    div()
                                        .flex_none()
                                        .w(gutter)
                                        .pr_3()
                                        .text_right()
                                        .text_color(colors.muted_foreground)
                                        .child((index + 1).to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(colors.foreground)
                                        .child(one_line(lines.line(index))),
                                )
                        })
                        .collect()
                })
                .track_scroll(&self.source)
                .size_full(),
            )
            .vertical_scrollbar(&self.source)
            .into_any_element()
    }

    fn footer(&self, colors: &Colors, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Body::Ready {
            format,
            picked,
            sheet,
            ..
        } = &self.body
        else {
            return None;
        };
        let mut facts = Vec::new();
        if let Some(sheet) = sheet {
            facts.push(m::file_rows(sheet.grid.rows()));
            facts.push(m::file_columns(sheet.grid.columns()));
        }
        if let Format::Delimited {
            separator,
            truncated,
        } = format
        {
            if *separator == ';' {
                facts.push(m::file_separator_semicolon());
            }
            if *truncated {
                facts.push(m::file_truncated(MAX_TEXT_BYTES / (1024 * 1024)));
            }
        }
        let tabs = match format {
            Format::Workbook { sheets } => sheets
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    let active = index == *picked;
                    let hover = colors.accent.opacity(0.6);
                    div()
                        .id(("file-sheet", index))
                        .debug_selector(move || format!("file-sheet-{index}"))
                        .flex()
                        .flex_none()
                        .items_center()
                        .px_3()
                        .h_full()
                        .border_r_1()
                        .border_color(colors.border)
                        .cursor_pointer()
                        .map(|tab| {
                            if active {
                                tab.bg(colors.secondary).text_color(colors.foreground)
                            } else {
                                tab.text_color(colors.muted_foreground)
                                    .hover(move |tab| tab.bg(hover))
                            }
                        })
                        .child(name.clone())
                        .on_click(cx.listener(move |this, _, _, cx| this.pick_sheet(index, cx)))
                        .into_any_element()
                })
                .collect(),
            Format::Delimited { .. } => Vec::new(),
        };
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .h_7()
                .border_t_1()
                .border_color(colors.border)
                .text_xs()
                .child(
                    div()
                        .id("file-sheets")
                        .flex()
                        .h_full()
                        .min_w_0()
                        .overflow_x_scroll()
                        .children(tabs),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .px_2p5()
                        .text_color(colors.muted_foreground)
                        .child(facts.join(" · ")),
                )
                .into_any_element(),
        )
    }
}

/// The bar atop a file: its folder and name, `controls`, then copying its path and opening it
/// in the system's app.
pub(super) fn path_bar(path: &str, colors: &Colors, controls: Vec<AnyElement>) -> Div {
    let (folder, name) = match path.rfind(['/', '\\']) {
        Some(slash) => path.split_at(slash + 1),
        None => ("", path),
    };
    let copied = if cfg!(windows) {
        path.replace('/', "\\")
    } else {
        path.to_owned()
    };
    let outside = path.to_owned();
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1p5()
        .border_b_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .text_xs()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(colors.muted_foreground)
                        .child(folder.to_owned()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(colors.foreground)
                        .child(name.to_owned()),
                ),
        )
        .children(controls)
        .child(
            Button::new("file-copy-path")
                .icon(IconName::Copy)
                .size(ButtonSize::IconSm)
                .variant(ButtonVariant::Ghost)
                .tooltip(Tooltip::new(m::file_copy_path()).side(Side::Bottom))
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                }),
        )
        .child(
            Button::new("file-open-outside")
                .icon(IconName::ExternalLink)
                .label(m::file_open_in_app())
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Outline)
                .on_click(move |_, _, cx| crate::markdown::open_outside(outside.clone(), cx)),
        )
}

fn loading(colors: &Colors) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .child(spinner(px(14.), colors.muted_foreground))
        .into_any_element()
}

fn sheet(grid: Grid, cx: &mut Context<FileSurface>) -> Sheet {
    let grid = Arc::new(grid);
    let view = cx.new(|cx| SheetGrid::new(grid.clone(), cx));
    Sheet { grid, view }
}

impl Render for FileSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let body = match &self.body {
            Body::Loading => loading(&colors),
            Body::Failed(failure)
            | Body::Ready {
                sheet_failure: Some(failure),
                ..
            } => panel_message(failure_message(failure), &colors).into_any_element(),
            Body::Ready { sheet: None, .. } => div().flex_1().into_any_element(),
            Body::Ready {
                sheet: Some(sheet), ..
            } if sheet.grid.rows() == 0 => {
                panel_message(m::file_empty(), &colors).into_any_element()
            }
            Body::Ready {
                format: Format::Delimited { .. },
                sheet: Some(sheet),
                ..
            } if self.view == View::Source => self.source_view(&sheet.grid, cx),
            Body::Ready {
                sheet: Some(sheet), ..
            } => sheet.view.clone().into_any_element(),
        };
        let toolbar = self.toolbar(&colors, cx);
        let footer = self.footer(&colors, cx);
        div()
            .debug_selector(|| "file-surface".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(body)
            .children(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::column_letters;

    #[test]
    fn columns_are_lettered_as_a_spreadsheet_letters_them() {
        assert_eq!(column_letters(0), "A");
        assert_eq!(column_letters(25), "Z");
        assert_eq!(column_letters(26), "AA");
        assert_eq!(column_letters(27), "AB");
        assert_eq!(column_letters(701), "ZZ");
        assert_eq!(column_letters(702), "AAA");
    }
}
