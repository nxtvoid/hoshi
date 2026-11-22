//! Markdown from agents and forges, shown and opened as the web app does: web links carry
//! their site's icon and open in the browser the settings pick, files on this machine open in
//! the editor, pull request links show the pull request on hover, and fenced code sits in a
//! card with its language and a copy button.

pub mod blocks;
mod file_links;
mod images;
mod links;

pub use images::{image as message_image, show as show_image, show_source as show_image_source};
pub mod mentions;
mod plain;
mod pull_request;
mod sites;

use crate::details::prefs;
use crate::m;
use crate::panel::{RightPanel, SurfaceKind};
use crate::state::Workspace;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::text::{
    MarkdownExtensions, MarkdownNode, SelectionFormat, TextView, TextViewStyle, markdown_ast,
};
use gpui_kit::*;
use hoshi_core::{Editor, LinkTarget};
use hoshi_engine::Engine;
use hoshi_ui::assets::{IconName, file_icon};
use hoshi_ui::prelude::{
    ActiveTheme as _, Button, ButtonSize, ButtonVariant, Colors, Tooltip, code_language_icon,
    code_language_label,
};
pub use plain::plain_text;
pub use pull_request::hover_card;
use pull_request::{LinkHovered, is_pull_request_url};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const CODE_BLOCK: &str = "hoshi-code-block";

/// What opening links needs: the engine, the home whose editor preference file links read,
/// and the workspace and panel a web link opens hoshi's browser in.
struct Links {
    engine: Arc<Engine>,
    home: PathBuf,
    workspace: Option<WeakEntity<Workspace>>,
    panel: Option<WeakEntity<RightPanel>>,
}

impl Global for Links {}

pub fn init(engine: Arc<Engine>, home: PathBuf, cx: &mut App) {
    sites::init(cx);
    cx.set_global(Links {
        engine,
        home,
        workspace: None,
        panel: None,
    });
}

/// The window's workspace, once attached.
pub fn workspace(cx: &App) -> Option<Entity<Workspace>> {
    cx.try_global::<Links>()?.workspace.as_ref()?.upgrade()
}

/// The window's workspace and panel, where web links open when the settings say hoshi.
pub fn attach(workspace: &Entity<Workspace>, panel: &Entity<RightPanel>, cx: &mut App) {
    if let Some(links) = cx.try_global::<Links>() {
        let links = Links {
            engine: links.engine.clone(),
            home: links.home.clone(),
            workspace: Some(workspace.downgrade()),
            panel: Some(panel.downgrade()),
        };
        cx.set_global(links);
    }
}

/// Nothing: a link whose text already is its address needs no tooltip.
struct NoTip;

impl Render for NoTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct FencedCode {
    code: SharedString,
    language: SharedString,
}

/// The space between blocks of markdown.
pub const PARAGRAPH_GAP: Rems = rems(0.625);

/// Markdown as the transcript shows it: the web's spacing, inline code on a faint tint.
pub fn markdown(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    colors: &Colors,
) -> TextView {
    let text: SharedString = text.into();
    view(id, &text, colors, Vec::new(), false).selection_format(SelectionFormat::SourceAcrossStyles)
}

/// [`markdown`] for an answer in the transcript, as wide as its column: only the blocks in view
/// are built, so a long answer costs as little to scroll past as a short one.
pub fn answer_markdown(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    colors: &Colors,
) -> TextView {
    let text: SharedString = text.into();
    view(id, &text, colors, Vec::new(), true).selection_format(SelectionFormat::SourceAcrossStyles)
}

/// The window's selected text as the clipboard gets it: markdown where it takes in several
/// styles, without what hoshi added to draw it.
pub fn selected_text(window: &mut Window, cx: &mut App) -> String {
    links::unprepare(gpui_kit::base::TextSelection::selected_text(window, cx).trim())
}

/// Copies the window's selected text; `false` when nothing is selected.
pub fn copy_selection(window: &mut Window, cx: &mut App) -> bool {
    let text = selected_text(window, cx);
    if text.is_empty() {
        return false;
    }
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    true
}

/// A message the user sent: its mentions as chips and its line breaks as written.
pub fn message_markdown(
    id: impl Into<ElementId>,
    text: &str,
    attachments: &[hoshi_core::MessageAttachment],
    colors: &Colors,
) -> TextView {
    let (tagged, mentions) = mentions::tag(text, &mentions::Known::of(attachments));
    view(id, &tagged, colors, mentions, false)
}

fn view(
    id: impl Into<ElementId>,
    text: &str,
    colors: &Colors,
    mentions: Vec<mentions::Mention>,
    lazy: bool,
) -> TextView {
    let mut extensions = MarkdownExtensions::default()
        .lazy_blocks(lazy)
        .block_parser(fenced_code)
        .block_renderer(CODE_BLOCK, render_code_block)
        .plugin(file_links::FileLinks);
    if !mentions.is_empty() {
        extensions = extensions.plugin(mentions::Mentions(Arc::new(mentions)));
    }
    let prepared = links::prepare(text);
    let labeled = prepared.labeled.clone();
    let link = colors.link;
    TextView::markdown(id, SharedString::from(prepared.text.to_string()))
        .selectable(true)
        .image_source(move |uri| {
            sites::resolve(uri, link)
                .or_else(|| local_path(uri).map(|path| ImageSource::from(PathBuf::from(path))))
                .unwrap_or_else(|| ImageSource::from(uri.clone()))
        })
        .style(TextViewStyle {
            paragraph_gap: PARAGRAPH_GAP,
            inline_code: HighlightStyle {
                background_color: Some(colors.foreground.opacity(0.1)),
                ..Default::default()
            },
            ..Default::default()
        })
        .on_link_click(|href, event, _, cx| {
            let modifiers = event.modifiers();
            open_link(href, modifiers.control || modifiers.platform, cx)
        })
        .link_tooltip(move |href, _, cx| {
            if is_pull_request_url(href) {
                cx.new(|_| LinkHovered(href.clone())).into()
            } else if labeled.contains(href.as_ref()) {
                cx.new(|_| LinkTip(href.clone())).into()
            } else {
                cx.new(|_| NoTip).into()
            }
        })
        .markdown_extensions(extensions)
}

fn fenced_code(
    node: &markdown_ast::Node,
    cx: &gpui_kit::component::text::MarkdownParseContext,
) -> Option<MarkdownNode> {
    let markdown_ast::Node::Code(code) = node else {
        return None;
    };
    let language = code.lang.clone().unwrap_or_default().to_lowercase();
    let source = cx.node_source(node).unwrap_or_default().to_owned();
    Some(
        MarkdownNode::new(
            CODE_BLOCK,
            FencedCode {
                code: code.value.clone().into(),
                language: language.into(),
            },
        )
        .text(code.value.clone())
        .markdown(source),
    )
}

fn render_code_block(node: &MarkdownNode, _: &mut Window, cx: &mut App) -> AnyElement {
    let Some(block) = node.data::<FencedCode>() else {
        return div().into_any_element();
    };
    let colors = cx.hoshi().clone();
    let at = node.source_range().map_or(0, |range| range.start);
    let (icon, tint) = code_language_icon(&block.language);
    let label: SharedString = if block.language.is_empty() {
        m::code_language_plain().into()
    } else {
        code_language_label(&block.language).into()
    };
    let fence = format!("```{}\n{}\n```", block.language, block.code);
    let mut code_style = StyleRefinement::default()
        .bg(transparent_black())
        .border_0()
        .rounded_none()
        .px_3()
        .pt_0()
        .pb_3();
    code_style.text.font_size = Some(cx.theme().mono_font_size.into());
    let code = block.code.clone();
    div()
        .my_1()
        .overflow_hidden()
        .rounded_lg()
        .border_1()
        .border_color(colors.border.opacity(0.7))
        .bg(colors.secondary.opacity(0.6))
        .child(
            div()
                .h_7()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .pl_3()
                .pr_1p5()
                .text_color(colors.muted_foreground)
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap_1p5()
                        .child(
                            svg()
                                .path(file_icon(icon))
                                .size_3p5()
                                .flex_none()
                                .text_color(tint(&colors)),
                        )
                        .child(div().truncate().text_size(px(11.)).child(label)),
                )
                .child(CopyButton::new(
                    ("copy-code", at),
                    code,
                    m::copy_code().into(),
                )),
        )
        .child(
            TextView::markdown(("code", at), fence)
                .selectable(true)
                .code_block_actions(|_, _, _| div())
                .style(TextViewStyle {
                    paragraph_gap: rems(0.),
                    code_block: code_style,
                    ..Default::default()
                }),
        )
        .into_any_element()
}

/// Copies its text, then shows a check for a moment.
#[derive(IntoElement)]
pub struct CopyButton {
    id: ElementId,
    text: SharedString,
    label: SharedString,
}

impl CopyButton {
    pub fn new(id: impl Into<ElementId>, text: SharedString, label: SharedString) -> Self {
        Self {
            id: id.into(),
            text,
            label,
        }
    }
}

impl RenderOnce for CopyButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let copied = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let shown = *copied.read(cx);
        let text = self.text;
        let label = self.label;
        Button::new(self.id)
            .icon(if shown {
                IconName::Check
            } else {
                IconName::Copy
            })
            .size(ButtonSize::IconXs)
            .variant(ButtonVariant::Ghost)
            .tooltip(Tooltip::new(if shown { m::copied().into() } else { label }))
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                copied.update(cx, |copied, cx| {
                    *copied = true;
                    cx.notify();
                });
                let copied = copied.downgrade();
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                    copied
                        .update(cx, |copied, cx| {
                            *copied = false;
                            cx.notify();
                        })
                        .ok();
                })
                .detach();
            })
    }
}

/// Where a link goes, shown while the pointer rests on it, in the tooltips' look.
struct LinkTip(SharedString);

impl Render for LinkTip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let target = local_path(&self.0).map_or_else(|| self.0.clone(), SharedString::from);
        clear_of_cursor(
            div()
                .max_w(rems(24.))
                .px(rems(0.75))
                .py(rems(0.375))
                .rounded(rems(0.5))
                .bg(colors.foreground)
                .text_color(colors.background)
                .text_size(rems(0.75))
                .line_height(rems(1.))
                .whitespace_normal()
                .child(target),
            cx,
        )
    }
}

/// Room between the pointer and a link's tooltip, which the window places at the pointer's tip.
/// It is drawn apart from the window, so it takes the interface font itself.
pub(super) fn clear_of_cursor(tip: impl IntoElement, cx: &App) -> Div {
    div()
        .pt(px(22.))
        .pl(px(6.))
        .pb(px(6.))
        .font_family(cx.theme().font_family.clone())
        .child(tip)
}

/// Opens a web link in hoshi's browser, beside the shown thread, when the settings say so and
/// Ctrl isn't held; in the default browser otherwise.
pub fn open_web_link(href: &str, external: bool, cx: &mut App) {
    let web = href.starts_with("https://") || href.starts_with("http://");
    let in_hoshi = workspace(cx).is_some_and(|workspace| {
        workspace.read(cx).settings.browser.open_links_in == LinkTarget::Hoshi
    });
    if !(in_hoshi && web && !external && open_in_panel(href, cx)) {
        cx.open_url(href);
    }
}

/// Opens `address` in hoshi's browser, beside the shown thread; `false` when no thread shows.
pub fn open_in_panel(address: &str, cx: &mut App) -> bool {
    let target = cx.try_global::<Links>().and_then(|links| {
        let workspace = links.workspace.as_ref()?.upgrade()?;
        let panel = links.panel.as_ref()?.upgrade()?;
        let thread_id = workspace.read(cx).open_thread_id(cx)?;
        Some((workspace, panel, thread_id))
    });
    let Some((workspace, panel, thread_id)) = target else {
        return false;
    };
    let browsers = workspace.read(cx).browsers.clone();
    panel.update(cx, |panel, cx| {
        panel.open_surface(&thread_id, SurfaceKind::Browser, cx)
    });
    browsers.update(cx, |browsers, cx| browsers.open(&thread_id, address, cx));
    true
}

fn open_link(href: &str, external: bool, cx: &mut App) {
    if href.starts_with('#') {
        return;
    }
    let Some(path) = local_path(href) else {
        open_web_link(href, external, cx);
        return;
    };
    open_local(path, cx);
}

/// Opens a file on this machine: code in the editor of "Open in", the rest in its app.
pub fn open_local(path: String, cx: &mut App) {
    let Some(links) = cx.try_global::<Links>() else {
        return;
    };
    let engine = links.engine.clone();
    let home = links.home.clone();
    cx.background_spawn(async move {
        let editor = opens_in_editor(&path)
            .then(|| code_editor(&engine.editors(), prefs::load(&home).preferred_editor))
            .flatten();
        if let Err(error) = engine.open_local_file(&path, editor) {
            eprintln!("hoshi: could not open {path}: {error:?}");
        }
    })
    .detach();
}

/// The code editor files open in: the one picked for "Open in", else the first installed.
fn code_editor(installed: &[Editor], preferred: Option<Editor>) -> Option<Editor> {
    let editors: Vec<Editor> = installed
        .iter()
        .copied()
        .filter(|editor| *editor != Editor::FileManager)
        .collect();
    prefs::open_target(
        &editors,
        preferred.filter(|editor| *editor != Editor::FileManager),
    )
}

/// Files the system opens better than a code editor.
fn opens_in_editor(path: &str) -> bool {
    const MEDIA: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "bmp", "ico", "pdf", "html", "htm",
        "mp4", "mov", "webm", "mp3", "wav",
    ];
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase());
    !extension.is_some_and(|extension| MEDIA.contains(&extension.as_str()))
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

fn is_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

fn is_posix_path(path: &str) -> bool {
    path.starts_with('/') && !path.starts_with("//")
}

/// The file on this machine a link names: an absolute path or a `file:` URL, without the
/// `:line` an agent may add. `None` for anything else.
pub fn local_path(url: &str) -> Option<String> {
    if url.contains(['\n', '\r']) {
        return None;
    }
    let path = if url.len() >= 7 && url[..7].eq_ignore_ascii_case("file://") {
        let path = percent_decoded(&url[7..]);
        let bytes = path.as_bytes();
        if bytes.len() >= 4
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':'
            && bytes[3] == b'/'
        {
            path[1..].to_owned()
        } else {
            path
        }
    } else if is_windows_path(url) || is_posix_path(url) {
        percent_decoded(url)
    } else {
        return None;
    };
    if !is_windows_path(&path) && !is_posix_path(&path) {
        return None;
    }
    Some(without_line(&path).to_owned())
}

fn without_line(path: &str) -> &str {
    let mut end = path;
    for _ in 0..2 {
        match end.rsplit_once(':') {
            Some((head, digits))
                if !digits.is_empty()
                    && digits.bytes().all(|byte| byte.is_ascii_digit())
                    && head.len() > 2 =>
            {
                end = head;
            }
            _ => break,
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::{local_path, opens_in_editor};

    #[test]
    fn local_paths_are_absolute_paths_and_file_urls() {
        assert_eq!(
            local_path("C:/Users/dev/src/main.rs:12:4").as_deref(),
            Some("C:/Users/dev/src/main.rs")
        );
        assert_eq!(
            local_path("file:///C:/My%20Docs/a.md").as_deref(),
            Some("C:/My Docs/a.md")
        );
        assert_eq!(
            local_path("/home/dev/notes.txt:3").as_deref(),
            Some("/home/dev/notes.txt")
        );
        assert_eq!(local_path("https://bitbucket.org/a"), None);
        assert_eq!(local_path("//cdn.example.com/x.png"), None);
        assert_eq!(local_path("#section"), None);
        assert!(opens_in_editor("C:/a/b.rs"));
        assert!(!opens_in_editor("C:/a/b.PNG"));
    }
}

#[cfg(test)]
mod ui_tests {
    use super::markdown;
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext,
        VisualTestContext, Window, div, px,
    };
    use hoshi_ui::prelude::ActiveTheme as _;

    struct View;

    impl Render for View {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let colors = cx.hoshi().clone();
            div().w(px(600.)).child(markdown(
                "message",
                "See [the repo](https://bitbucket.org/a/b), [main.rs](C:/a/main.rs:3) and `+review`.\n\n1. Step\n   ```rust\n   fn main() {}\n   ```\n\n```\nplain\n```",
                &colors,
            ))
        }
    }

    #[gpui_kit::test]
    fn fenced_code_renders_in_its_card(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        });
        let window = cx.add_window(|_, _| View);
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        assert!(cx.debug_bounds("tooltip").is_none());
    }
}

#[cfg(test)]
mod emphasis_tests {
    use gpui_kit::base::text::editor_host::{BlockNode, parse_markdown};
    use gpui_kit::component::text::MarkdownExtensions;
    use std::sync::Arc;

    fn runs(text: &str) -> Vec<(String, bool, bool)> {
        let prepared = super::links::prepare(text);
        let extensions = MarkdownExtensions::default()
            .block_parser(super::fenced_code)
            .plugin(super::file_links::FileLinks);
        let blocks = parse_markdown(&prepared.text, Arc::new(extensions)).unwrap();
        let mut runs = Vec::new();
        for block in blocks {
            if let BlockNode::Paragraph(paragraph) = block {
                for inline in paragraph.children() {
                    let mark = inline.mark_runs().first().map(|(_, mark)| mark.clone());
                    runs.push((
                        inline.text().to_owned(),
                        mark.as_ref().is_some_and(|mark| mark.italic),
                        mark.as_ref().is_some_and(|mark| mark.bold),
                    ));
                }
            }
        }
        runs
    }

    #[test]
    fn underscores_inside_words_stay_literal() {
        for text in [
            "Use html_preview and html_render.",
            "snake_case_name",
            "Open file_name.rs and other_file.rs",
            "mcp__hoshi__html_preview",
        ] {
            assert_eq!(runs(text), vec![(text.to_owned(), false, false)], "{text}");
        }
    }

    #[test]
    fn emphasis_between_words_still_marks() {
        assert_eq!(
            runs("a _real_ *one*"),
            vec![
                ("a ".to_owned(), false, false),
                ("real".to_owned(), true, false),
                (" ".to_owned(), false, false),
                ("one".to_owned(), true, false),
            ]
        );
        assert_eq!(
            runs("__init__.py"),
            vec![
                ("init".to_owned(), false, true),
                (".py".to_owned(), false, false),
            ]
        );
    }
}

#[cfg(test)]
mod parse_tests {
    use gpui_kit::base::text::editor_host::{BlockNode, parse_markdown};
    use gpui_kit::component::text::MarkdownExtensions;
    use std::sync::Arc;

    fn blocks(text: &str) -> Vec<BlockNode> {
        let extensions = MarkdownExtensions::default()
            .block_parser(super::fenced_code)
            .plugin(super::file_links::FileLinks);
        parse_markdown(text, Arc::new(extensions)).expect("the text parses")
    }

    #[test]
    fn a_message_the_parser_cant_read_shows_as_its_lines() {
        for text in ["- ```\n1)", "+ $$\n1)", "- ```\n2.", "0) <?\n-"] {
            assert_eq!(&*super::links::prepare(text).text, text);
            let lines: Vec<String> = blocks(text)
                .iter()
                .filter_map(|block| match block {
                    BlockNode::Paragraph(paragraph) => Some(
                        paragraph
                            .children()
                            .iter()
                            .map(|inline| inline.text().to_owned())
                            .collect(),
                    ),
                    _ => None,
                })
                .collect();
            let expected: Vec<&str> = text.lines().collect();
            assert_eq!(lines, expected, "{text:?}");
        }
    }

    #[test]
    fn random_text_never_panics_the_parser() {
        const FRAGMENTS: &[&str] = &[
            "\n",
            "\n",
            "- ",
            "- a\n",
            "1. x\n",
            "2.",
            "0) ",
            "1)",
            "+ ",
            "<?",
            "<pre>\n",
            "```",
            "```\n",
            "~~~\n",
            "$$\n",
            "===\n",
            "---\n",
            "> ",
            "> q\n",
            "[a",
            "]: u\n",
            "[^1]: n\n",
            "    i\n",
            "  ",
            "| a |\n",
            "| - |\n",
            "*",
            "**",
            "`",
            "x\n",
            "# h\n",
            "<!--\n",
            "-->",
            "![i](",
            ")",
            "\t",
            "<div>\n",
            "</div>\n",
            "[x](y)",
            "https://a.io ",
            "\\",
            "&amp;",
        ];
        let mut seed: u64 = 0x1234_5678_9abc_def1;
        let mut next = move |below: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % below as u64) as usize
        };
        for _ in 0..20_000 {
            let text: String = (0..2 + next(8))
                .map(|_| FRAGMENTS[next(FRAGMENTS.len())])
                .collect();
            super::links::prepare(&text);
            blocks(&text);
        }
    }
}
