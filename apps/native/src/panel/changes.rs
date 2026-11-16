use super::panel_message;
use crate::m;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::*;
use hoshi_core::{CommandError, DiffFile, DiffFileStatus, DiffScope, FileDiff};
use hoshi_engine::Engine;
use hoshi_ui::prelude::*;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

/// Where a list of changed files comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Thread(DiffScope),
    Turn(String),
    PullRequest,
}

impl Source {
    fn files(
        &self,
        engine: &Engine,
        thread_id: &str,
    ) -> Result<Vec<DiffFile>, Option<CommandError>> {
        match self {
            Self::Thread(scope) => engine.thread_diff_files(thread_id, *scope).map_err(Some),
            Self::Turn(turn_id) => engine.turn_diff_files(thread_id, turn_id).map_err(Some),
            Self::PullRequest => engine.pull_request_files(thread_id).ok_or(None),
        }
    }

    /// `Ok(None)` when the file has no diff to show, such as a binary.
    fn diff(
        &self,
        engine: &Engine,
        thread_id: &str,
        file: &DiffFile,
    ) -> Result<Option<FileDiff>, ()> {
        match self {
            Self::Thread(scope) => engine
                .thread_file_diff(thread_id, *scope, file)
                .map(Some)
                .map_err(drop),
            Self::Turn(turn_id) => engine
                .turn_file_diff(thread_id, turn_id, file)
                .map(Some)
                .map_err(drop),
            Self::PullRequest => Ok(engine.pull_request_file_diff(thread_id, &file.path)),
        }
    }
}

pub enum Files {
    Loading,
    Ready(Vec<DiffFile>),
    Failed(Option<CommandError>),
}

pub enum FileDiffState {
    Ready {
        lines: Rc<Vec<DiffLine>>,
        truncated: bool,
    },
    Unavailable,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    Failed,
    Unavailable,
    Truncated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    File,
    Line(usize),
    Note(Note),
}

/// One row of the list: a file's header, a line of its diff, or a note under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub path: Rc<str>,
    pub kind: RowKind,
}

/// The rows the files show, each open one followed by its diff once loaded.
pub fn build_rows(
    files: &[DiffFile],
    open: &HashSet<String>,
    diffs: &HashMap<String, FileDiffState>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for file in files {
        let path: Rc<str> = file.path.as_str().into();
        let row = |kind| Row {
            path: path.clone(),
            kind,
        };
        rows.push(row(RowKind::File));
        if !open.contains(&file.path) {
            continue;
        }
        match diffs.get(&file.path) {
            None => {}
            Some(FileDiffState::Failed) => rows.push(row(RowKind::Note(Note::Failed))),
            Some(FileDiffState::Unavailable) => {
                rows.push(row(RowKind::Note(Note::Unavailable)));
            }
            Some(FileDiffState::Ready { lines, truncated }) => {
                if lines.is_empty() {
                    rows.push(row(RowKind::Note(Note::Unavailable)));
                }
                rows.extend((0..lines.len()).map(|line| row(RowKind::Line(line))));
                if *truncated {
                    rows.push(row(RowKind::Note(Note::Truncated)));
                }
            }
        }
    }
    rows
}

/// The rows of `old` to replace and how many of `new` take their place; `None` when equal.
pub fn changed_rows<T: PartialEq>(old: &[T], new: &[T]) -> Option<(Range<usize>, usize)> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    if prefix == old.len() && prefix == new.len() {
        return None;
    }
    let room = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    Some((prefix..old.len() - suffix, new.len() - prefix - suffix))
}

pub fn totals(files: &[DiffFile]) -> (u32, u32) {
    files.iter().fold((0, 0), |(additions, deletions), file| {
        (
            additions + file.additions.unwrap_or(0),
            deletions + file.deletions.unwrap_or(0),
        )
    })
}

/// Changed files that each open to their diff, loaded only when opened. One virtual list for
/// every file, so long diffs only build the lines in view.
pub struct ChangedFiles {
    engine: Arc<Engine>,
    thread_id: String,
    source: Source,
    files: Files,
    positions: HashMap<String, usize>,
    open: HashSet<String>,
    diffs: HashMap<String, FileDiffState>,
    rows: Vec<Row>,
    list: ListState,
    generation: usize,
    /// Opened and scrolled into view when the files arrive.
    focus_path: Option<String>,
    /// How far each open file's diff is scrolled sideways when lines don't wrap.
    scroll_x: HashMap<String, Pixels>,
    /// Each open file's widest line with its numbers, by the font it was measured in.
    widths: HashMap<String, (String, Pixels)>,
    /// The scrollbar thumb being dragged: its file, where the drag began and the scroll then.
    thumb: Option<(Rc<str>, Pixels, Pixels)>,
    /// The list's width when it last drew; the list can't be asked while it builds rows.
    viewport: Pixels,
}

/// What a sideways scrollbar thumb drags.
struct ThumbDrag;

/// Where diff lines sit in the window's reading order: after anything ordered by painting,
/// such as the transcript, then by file and line.
const SELECTION_ORDER: u64 = 1 << 48;

/// The scroll an offset may reach for content `content` wide in a `viewport` wide view.
pub fn max_scroll(content: Pixels, viewport: Pixels) -> Pixels {
    (content - viewport).max(px(0.))
}

/// Where a sideways scroll ends after a wheel `delta`: the content follows the wheel.
pub fn scrolled(offset: Pixels, delta: Pixels, max: Pixels) -> Pixels {
    (offset - delta).clamp(px(0.), max)
}

/// The thumb of a sideways scrollbar as its left edge and width in a `viewport` wide track.
pub fn thumb(offset: Pixels, content: Pixels, viewport: Pixels) -> (Pixels, Pixels) {
    let width = (viewport * (viewport / content)).max(px(24.)).min(viewport);
    let max = max_scroll(content, viewport);
    let left = if max > px(0.) {
        (viewport - width) * (offset / max)
    } else {
        px(0.)
    };
    (left, width)
}

impl ChangedFiles {
    pub fn new(
        engine: Arc<Engine>,
        thread_id: String,
        source: Source,
        focus_path: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            engine,
            thread_id,
            source,
            files: Files::Loading,
            positions: HashMap::new(),
            open: HashSet::new(),
            diffs: HashMap::new(),
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            generation: 0,
            focus_path,
            scroll_x: HashMap::new(),
            widths: HashMap::new(),
            thumb: None,
            viewport: px(0.),
        };
        this.reload(cx);
        this
    }

    pub fn files(&self) -> &Files {
        &self.files
    }

    /// Shows another list, as a new one: nothing open, from the top.
    pub fn show(&mut self, source: Source, focus_path: Option<String>, cx: &mut Context<Self>) {
        if source == self.source && focus_path == self.focus_path {
            return;
        }
        self.source = source;
        self.focus_path = focus_path;
        self.files = Files::Loading;
        self.positions.clear();
        self.open.clear();
        self.diffs.clear();
        self.scroll_x.clear();
        self.widths.clear();
        self.rows.clear();
        self.list.reset(0);
        self.reload(cx);
    }

    /// Reads the files again, and the diffs that are open, keeping what shows meanwhile.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let (engine, thread_id, source) = (
            self.engine.clone(),
            self.thread_id.clone(),
            self.source.clone(),
        );
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn(async move { source.files(&engine, &thread_id) })
                .await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.receive_files(files, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn receive_files(
        &mut self,
        files: Result<Vec<DiffFile>, Option<CommandError>>,
        cx: &mut Context<Self>,
    ) {
        let files = match files {
            Ok(files) => files,
            Err(error) => {
                self.files = Files::Failed(error);
                self.rebuild(cx);
                return;
            }
        };
        self.positions = files
            .iter()
            .enumerate()
            .map(|(index, file)| (file.path.clone(), index))
            .collect();
        let focus = self
            .focus_path
            .take()
            .filter(|path| self.positions.contains_key(path));
        if let Some(path) = &focus {
            self.open.insert(path.clone());
        }
        self.open.retain(|path| self.positions.contains_key(path));
        self.diffs
            .retain(|path, _| self.positions.contains_key(path));
        self.scroll_x
            .retain(|path, _| self.positions.contains_key(path));
        let open: Vec<DiffFile> = files
            .iter()
            .filter(|file| self.open.contains(&file.path))
            .cloned()
            .collect();
        self.files = Files::Ready(files);
        for file in open {
            self.load_diff(file, cx);
        }
        self.rebuild(cx);
        if let Some(path) = focus
            && let Some(index) = self.header_index(&path)
        {
            self.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
        }
    }

    fn load_diff(&mut self, file: DiffFile, cx: &mut Context<Self>) {
        let generation = self.generation;
        let (engine, thread_id, source) = (
            self.engine.clone(),
            self.thread_id.clone(),
            self.source.clone(),
        );
        let path = file.path.clone();
        cx.spawn(async move |this, cx| {
            let diff = cx
                .background_spawn(async move {
                    source.diff(&engine, &thread_id, &file).map(|diff| {
                        diff.map(|diff| (parse_unified_diff(&diff.text), diff.truncated))
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.generation != generation || !this.open.contains(&path) {
                    return;
                }
                let state = match diff {
                    Ok(Some((lines, truncated))) => FileDiffState::Ready {
                        lines: Rc::new(lines),
                        truncated,
                    },
                    Ok(None) => FileDiffState::Unavailable,
                    Err(()) => FileDiffState::Failed,
                };
                this.diffs.insert(path.clone(), state);
                this.widths.remove(&path);
                this.rebuild(cx);
                if let Some(range) = this.file_rows(&path) {
                    this.list.remeasure_items(range);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn toggle(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.open.remove(path) {
            self.diffs.remove(path);
            self.widths.remove(path);
            self.scroll_x.remove(path);
        } else if let Files::Ready(files) = &self.files
            && let Some(file) = files.iter().find(|file| file.path == path).cloned()
        {
            self.open.insert(path.to_owned());
            self.load_diff(file, cx);
        }
        self.rebuild(cx);
    }

    #[cfg(test)]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    #[cfg(test)]
    pub fn scroll_of(&self, path: &str) -> Pixels {
        self.scroll_x.get(path).copied().unwrap_or_default()
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let rows = match &self.files {
            Files::Ready(files) => build_rows(files, &self.open, &self.diffs),
            _ => Vec::new(),
        };
        if let Some((range, count)) = changed_rows(&self.rows, &rows) {
            self.list.splice(range, count);
        }
        self.rows = rows;
        cx.notify();
    }

    fn header_index(&self, path: &str) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.kind == RowKind::File && &*row.path == path)
    }

    fn file_rows(&self, path: &str) -> Option<Range<usize>> {
        let start = self.header_index(path)?;
        let len = self.rows[start..]
            .iter()
            .take_while(|row| &*row.path == path)
            .count();
        Some(start..start + len)
    }

    /// How wide the file's widest line draws, numbers and sign included.
    fn content_width(&mut self, path: &str, window: &mut Window, cx: &App) -> Pixels {
        let theme = cx.theme();
        let (family, size) = (theme.mono_font_family.clone(), theme.mono_font_size);
        let key = format!("{family}:{size:?}:{:?}", window.rem_size());
        if let Some((measured, width)) = self.widths.get(path)
            && *measured == key
        {
            return *width;
        }
        let Some(FileDiffState::Ready { lines, .. }) = self.diffs.get(path) else {
            return px(0.);
        };
        let widest = lines
            .iter()
            .max_by_key(|line| line.text.chars().count())
            .map(|line| line.text.clone())
            .unwrap_or_default();
        let run = TextRun {
            len: widest.len(),
            font: font(family),
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let text = window
            .text_system()
            .shape_line(widest.into(), size, &[run], None)
            .width;
        let width = (text + window.rem_size() * 6.5).ceil() + px(2.);
        self.widths.insert(path.to_owned(), (key, width));
        width
    }

    fn sideways(&self, path: &str) -> (Pixels, Pixels) {
        let viewport = self.viewport;
        let content = self.widths.get(path).map_or(px(0.), |(_, width)| *width);
        (content, viewport)
    }

    fn scroll_sideways(&mut self, path: &str, event: &ScrollEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(20.));
        let delta = if delta.x != px(0.) {
            delta.x
        } else if event.modifiers.shift {
            delta.y
        } else {
            return;
        };
        let (content, viewport) = self.sideways(path);
        let max = max_scroll(content, viewport);
        let offset = self.scroll_x.get(path).copied().unwrap_or_default();
        let next = scrolled(offset, delta, max);
        cx.stop_propagation();
        if next != offset {
            self.scroll_x.insert(path.to_owned(), next);
            cx.notify();
        }
    }

    fn drag_thumb(&mut self, pointer: Pixels, cx: &mut Context<Self>) {
        let Some((path, start, from)) = self.thumb.clone() else {
            return;
        };
        let (content, viewport) = self.sideways(&path);
        let max = max_scroll(content, viewport);
        let (_, width) = thumb(from, content, viewport);
        let track = viewport - width;
        if track <= px(0.) {
            return;
        }
        let next = (from + (pointer - start) * (max / track)).clamp(px(0.), max);
        self.scroll_x.insert(path.to_string(), next);
        cx.notify();
    }

    /// One line of a file's diff; without wrapping it is as wide as the widest line and moves
    /// sideways with the rest of the file.
    fn line_row(
        &mut self,
        path: Rc<str>,
        line: DiffLine,
        index: usize,
        last: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let file = self.positions.get(&*path).copied().unwrap_or_default() as u64;
        let row = DiffLineRow::new(line).selectable(
            SharedString::from(format!("{path}:{index}")),
            SELECTION_ORDER + (file << 24) + index as u64,
        );
        if hoshi_ui::theme::fonts(cx).word_wrap {
            return div().child(row).into_any_element();
        }
        let content = self.content_width(&path, window, cx);
        let (_, viewport) = self.sideways(&path);
        let max = max_scroll(content, viewport);
        let offset = self
            .scroll_x
            .get(&*path)
            .copied()
            .unwrap_or_default()
            .min(max);
        let colors = cx.hoshi().clone();
        let wheel = path.clone();
        div()
            .id(SharedString::from(format!("diff-row:{path}:{index}")))
            .relative()
            .when(last && max > px(0.), |row| row.pb(px(8.)))
            .on_scroll(cx.listener(move |this, event: &ScrollEvent, _, cx| {
                this.scroll_sideways(&wheel, event, cx)
            }))
            .child(
                div()
                    .relative()
                    .left(-offset)
                    .flex_none()
                    .w(content.max(viewport))
                    .child(row),
            )
            .when(last && max > px(0.), |row| {
                let (left, width) = thumb(offset, content, viewport);
                let dragged = path.clone();
                let selector = format!("diff-thumb-{path}");
                row.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(px(1.))
                        .h(px(6.))
                        .child(
                            div()
                                .id(SharedString::from(format!("diff-thumb:{path}")))
                                .debug_selector(move || selector)
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(left)
                                .w(width)
                                .rounded_full()
                                .bg(colors.scrollbar)
                                .hover(|thumb| thumb.bg(colors.scrollbar_hover))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                        gpui_kit::base::GlobalState::suppress_text_selection(cx);
                                        this.thumb =
                                            Some((dragged.clone(), event.position.x, offset));
                                    }),
                                )
                                .on_drag(ThumbDrag, |_, _, _, cx| cx.new(|_| EmptyView)),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.hoshi().clone();
        let (Some(row), Files::Ready(files)) = (self.rows.get(index).cloned(), &self.files) else {
            return div().into_any_element();
        };
        let last = self
            .rows
            .get(index + 1)
            .is_none_or(|next| next.path != row.path);
        let body = match row.kind {
            RowKind::File => {
                let Some(file) = self.positions.get(&*row.path).and_then(|at| files.get(*at))
                else {
                    return div().into_any_element();
                };
                self.file_header(index, file, &colors, cx)
            }
            RowKind::Line(line) => match self.diffs.get(&*row.path) {
                Some(FileDiffState::Ready { lines, .. }) if line < lines.len() => {
                    let last_line = line + 1 == lines.len();
                    let diff_line = lines[line].clone();
                    div()
                        .when(last_line, |row| row.pb_1())
                        .when(line == 0, |row| row.pt_0p5())
                        .child(self.line_row(
                            row.path.clone(),
                            diff_line,
                            line,
                            last_line,
                            window,
                            cx,
                        ))
                        .into_any_element()
                }
                _ => div().into_any_element(),
            },
            RowKind::Note(note) => panel_message(
                match note {
                    Note::Failed => m::diff_load_failed(),
                    Note::Unavailable => m::diff_unavailable(),
                    Note::Truncated => m::diff_truncated(),
                },
                &colors,
            )
            .into_any_element(),
        };
        div()
            .when(last, |row| {
                row.border_b_1().border_color(colors.border.opacity(0.6))
            })
            .child(body)
            .into_any_element()
    }

    fn file_header(
        &self,
        index: usize,
        file: &DiffFile,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.open.contains(&file.path);
        let (folder, name) = match file.path.rfind('/') {
            Some(slash) => file.path.split_at(slash + 1),
            None => ("", file.path.as_str()),
        };
        let name_color = match file.status {
            DiffFileStatus::Added | DiffFileStatus::Untracked => colors.pr_open,
            DiffFileStatus::Deleted => colors.status_error,
            DiffFileStatus::Modified | DiffFileStatus::Renamed => colors.foreground,
        };
        let path = file.path.clone();
        let selector = format!("changed-file-{}", file.path);
        let mono = cx.theme().mono_font_family.clone();
        div()
            .id(("changed-file", index))
            .debug_selector(move || selector)
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1p5()
            .text_xs()
            .cursor_pointer()
            .hover(|row| row.bg(colors.accent.opacity(0.6)))
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size_3()
                .flex_none()
                .text_color(colors.muted_foreground),
            )
            .child(Icon::new(IconName::File).size_3p5().flex_none())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .font_family(mono)
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
                            .text_color(name_color)
                            .child(name.to_owned()),
                    ),
            )
            .map(|row| match (file.additions, file.deletions) {
                (None, _) => row.child(
                    div()
                        .flex_none()
                        .text_color(colors.muted_foreground)
                        .child(m::diff_binary()),
                ),
                (Some(additions), deletions) => {
                    row.child(DiffStat::new(additions, deletions.unwrap_or(0)))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(&path, cx)))
            .into_any_element()
    }
}

impl Render for ChangedFiles {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.viewport = self.list.viewport_bounds().size.width;
        let colors = cx.hoshi().clone();
        let body = match &self.files {
            Files::Loading => None,
            Files::Failed(Some(CommandError::NotAGitRepository { .. })) => {
                Some(panel_message(m::diff_not_a_repository(), &colors))
            }
            Files::Failed(_) => Some(panel_message(m::diff_load_failed(), &colors)),
            Files::Ready(files) if files.is_empty() => {
                Some(panel_message(m::no_changes(), &colors))
            }
            Files::Ready(_) => None,
        };
        let ready = matches!(&self.files, Files::Ready(files) if !files.is_empty());
        div()
            .id("changed-files")
            .relative()
            .flex_1()
            .min_h_0()
            .size_full()
            .on_drag_move::<ThumbDrag>(cx.listener(
                |this, event: &DragMoveEvent<ThumbDrag>, _, cx| {
                    this.drag_thumb(event.event.position.x, cx)
                },
            ))
            .children(body)
            .when(ready, |this| {
                this.child(list(self.list.clone(), cx.processor(Self::render_row)).size_full())
                    .vertical_scrollbar(&self.list)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{FileDiffState, Note, Row, RowKind, build_rows, changed_rows, totals};
    use hoshi_core::{DiffFile, DiffFileStatus};
    use hoshi_ui::{DiffLine, DiffLineKind};
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    fn file(path: &str, additions: Option<u32>) -> DiffFile {
        DiffFile {
            path: path.into(),
            old_path: None,
            status: DiffFileStatus::Modified,
            additions,
            deletions: additions.map(|_| 1),
        }
    }

    fn lines(count: usize) -> Rc<Vec<DiffLine>> {
        Rc::new(
            (0..count)
                .map(|index| DiffLine {
                    kind: DiffLineKind::Context,
                    text: index.to_string(),
                    old_line: Some(index as u32),
                    new_line: Some(index as u32),
                })
                .collect(),
        )
    }

    fn kinds(rows: &[Row]) -> Vec<(&str, RowKind)> {
        rows.iter().map(|row| (&*row.path, row.kind)).collect()
    }

    #[test]
    fn an_open_file_shows_its_lines_once_loaded_and_a_note_when_cut_short() {
        let files = [
            file("a.rs", Some(2)),
            file("b.rs", Some(1)),
            file("c.bin", None),
        ];
        let open: HashSet<String> = ["a.rs", "b.rs", "c.bin"].map(String::from).into();
        let mut diffs = HashMap::new();
        diffs.insert(
            "a.rs".to_owned(),
            FileDiffState::Ready {
                lines: lines(2),
                truncated: true,
            },
        );
        diffs.insert("c.bin".to_owned(), FileDiffState::Unavailable);

        let rows = build_rows(&files, &open, &diffs);

        assert_eq!(
            kinds(&rows),
            [
                ("a.rs", RowKind::File),
                ("a.rs", RowKind::Line(0)),
                ("a.rs", RowKind::Line(1)),
                ("a.rs", RowKind::Note(Note::Truncated)),
                ("b.rs", RowKind::File),
                ("c.bin", RowKind::File),
                ("c.bin", RowKind::Note(Note::Unavailable)),
            ]
        );
    }

    #[test]
    fn a_closed_file_is_only_its_header_and_an_empty_diff_says_so() {
        let files = [file("a.rs", Some(2)), file("b.rs", Some(1))];
        let open: HashSet<String> = ["b.rs"].map(String::from).into();
        let mut diffs = HashMap::new();
        diffs.insert(
            "a.rs".to_owned(),
            FileDiffState::Ready {
                lines: lines(3),
                truncated: false,
            },
        );
        diffs.insert(
            "b.rs".to_owned(),
            FileDiffState::Ready {
                lines: lines(0),
                truncated: false,
            },
        );

        assert_eq!(
            kinds(&build_rows(&files, &open, &diffs)),
            [
                ("a.rs", RowKind::File),
                ("b.rs", RowKind::File),
                ("b.rs", RowKind::Note(Note::Unavailable)),
            ]
        );
    }

    #[test]
    fn only_the_rows_between_the_shared_ends_change() {
        assert_eq!(changed_rows(&[1, 2, 3], &[1, 2, 3]), None);
        assert_eq!(changed_rows(&[1, 4], &[1, 2, 3, 4]), Some((1..1, 2)));
        assert_eq!(changed_rows(&[1, 2, 3, 4], &[1, 4]), Some((1..3, 0)));
        assert_eq!(changed_rows(&[1, 1], &[1, 1, 1]), Some((2..2, 1)));
        assert_eq!(changed_rows::<u8>(&[], &[5]), Some((0..0, 1)));
        assert_eq!(changed_rows(&[1, 2], &[3]), Some((0..2, 1)));
    }

    #[test]
    fn totals_skip_binaries() {
        assert_eq!(
            totals(&[file("a", Some(3)), file("b", None), file("c", Some(2))]),
            (5, 2)
        );
    }
}
