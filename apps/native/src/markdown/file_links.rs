//! A link to a file on this machine whose text names the file, drawn as a chip: the file's
//! icon and name, its full path and size on hover, and a click that opens it.

use super::{local_path, open_local};
use crate::composer::format_size;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::text::{InlineElement, InlineRenderContext, MarkdownNode};
use gpui_kit::component::text::{MarkdownParseContext, MarkdownPlugin, markdown_ast};
use gpui_kit::*;
use hoshi_ui::assets::file_icon;
use hoshi_ui::prelude::{
    ActiveTheme as _, PressSelectsNothing as _, Tooltip, WithTooltip as _, code_language_icon,
};
use markdown::mdast::Node;

const TAG: &str = "hoshi-file-link";

#[derive(Clone, Debug, PartialEq)]
pub struct FileLink {
    pub path: String,
    /// The path with the line the link points at, as the tooltip shows it.
    pub target: SharedString,
    pub label: SharedString,
}

fn plain_text(node: &Node) -> String {
    match node {
        Node::Text(text) => text.value.clone(),
        Node::InlineCode(code) => code.value.clone(),
        _ => node
            .children()
            .map(|children| children.iter().map(plain_text).collect())
            .unwrap_or_default(),
    }
}

fn has_image(node: &Node) -> bool {
    matches!(node, Node::Image(_))
        || node
            .children()
            .is_some_and(|children| children.iter().any(has_image))
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The line and column after a path, as `:12` or `:12:4`.
fn position(url: &str, path: &str) -> Option<(u32, Option<u32>)> {
    let tail = url.get(url.rfind(basename(path))? + basename(path).len()..)?;
    let mut numbers = tail.strip_prefix(':')?.split(':').map(str::parse::<u32>);
    let line = numbers.next()?.ok().filter(|line| *line > 0)?;
    let column = numbers
        .next()
        .and_then(Result::ok)
        .filter(|column| *column > 0);
    Some((line, column))
}

fn normalized(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_lowercase()
}

/// Whether the link's text is its file, whole or the end of its path.
fn names_file(label: &str, path: &str) -> bool {
    let label = normalized(super::without_line(label.trim()));
    let path = normalized(path);
    label.is_empty() || path == label || path.ends_with(&format!("/{label}"))
}

/// The chip a link stands for: a link to a local file that its text names.
pub fn file_link(link: &markdown::mdast::Link) -> Option<FileLink> {
    let node = Node::Link(link.clone());
    if has_image(&node) {
        return None;
    }
    let path = local_path(&link.url)?;
    let text = plain_text(&node);
    if text != link.url && !names_file(&text, &path) {
        return None;
    }
    Some(at(&link.url, path))
}

/// The chip inline code stands for when it is the absolute path of a file and nothing else.
pub fn code_path(code: &str) -> Option<FileLink> {
    let code = code.trim();
    let elided = code.split(['/', '\\']).any(|segment| {
        segment == "…" || (segment.len() >= 3 && segment.bytes().all(|byte| byte == b'.'))
    });
    if code.is_empty() || elided || code.contains(|char: char| char.is_whitespace() || char == '`')
    {
        return None;
    }
    let path = local_path(code)?;
    Some(at(code, path))
}

fn at(url: &str, path: String) -> FileLink {
    let position = position(url, &path);
    let label = match position {
        Some((line, Some(column))) => format!("{} · L{line}:C{column}", basename(&path)),
        Some((line, None)) => format!("{} · L{line}", basename(&path)),
        None => basename(&path).to_owned(),
    };
    let target = match position {
        Some((line, Some(column))) => format!("{path}:{line}:{column}"),
        Some((line, None)) => format!("{path}:{line}"),
        None => path.clone(),
    };
    FileLink {
        path,
        target: target.into(),
        label: label.into(),
    }
}

/// Turns links to local files into chips.
pub struct FileLinks;

impl MarkdownPlugin for FileLinks {
    fn name(&self) -> &str {
        TAG
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        _: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let (file, source) = match node {
            markdown_ast::Node::Link(link) => (
                file_link(link)?,
                format!("[{}]({})", plain_text(node), link.url),
            ),
            markdown_ast::Node::InlineCode(code) => {
                let path = code.value.trim_start_matches(super::links::BARE);
                (code_path(path)?, format!("`{}`", code.value))
            }
            _ => return None,
        };
        Some(
            MarkdownNode::new(TAG, file.clone())
                .text(file.label)
                .markdown(source),
        )
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        context: &InlineRenderContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let file = node.data::<FileLink>()?.clone();
        let line_height = context.line_height();
        let font_size = context.font_size();
        let at = node.source_range().map_or(0, |range| range.start);
        Some(
            InlineElement::new(chip(file, at, font_size, line_height, cx))
                .with_baseline(line_height / 2. + font_size * 0.35),
        )
    }
}

fn chip(
    file: FileLink,
    at: usize,
    font_size: Pixels,
    line_height: Pixels,
    cx: &App,
) -> impl IntoElement {
    let colors = cx.hoshi().clone();
    let extension = basename(&file.path)
        .rsplit_once('.')
        .map_or("", |(_, extension)| extension);
    let (icon, tint) = code_language_icon(extension);
    let path = file.path.clone();
    let target = file.target.clone();
    let mono = cx.theme().mono_font_family.clone();
    div()
        .id(("file-link", at))
        .h(line_height)
        .flex()
        .items_center()
        .gap(font_size * 0.2)
        .text_size(font_size)
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors.link)
        .whitespace_nowrap()
        .cursor_pointer()
        .child(
            svg()
                .path(file_icon(icon))
                .size(font_size)
                .flex_none()
                .text_color(tint(&colors)),
        )
        .child(
            div()
                .hover(|style| style.underline())
                .child(file.label.clone()),
        )
        .press_selects_nothing()
        .on_click(move |_, _, cx| open_local(path.clone(), cx))
        .with_tooltip(
            Tooltip::element(move |_, _| {
                let size = std::fs::metadata(super::without_line(&target))
                    .ok()
                    .filter(std::fs::Metadata::is_file)
                    .map(|metadata| format!(" · {}", format_size(metadata.len())));
                div()
                    .font_family(mono.clone())
                    .child(format!("{target}{}", size.unwrap_or_default()))
            })
            .max_width(rems(32.)),
        )
}

#[cfg(test)]
mod tests {
    use super::{FileLink, file_link};
    use markdown::mdast::Node;

    fn first_link(text: &str) -> Option<FileLink> {
        fn find(node: &Node) -> Option<&markdown::mdast::Link> {
            if let Node::Link(link) = node {
                return Some(link);
            }
            node.children()?.iter().find_map(find)
        }
        let tree = markdown::to_mdast(text, &markdown::ParseOptions::gfm()).ok()?;
        file_link(find(&tree)?)
    }

    #[test]
    fn links_named_after_their_file_become_chips() {
        let chip = first_link("See [ui.html](C:/Users/dev/tmp/ui.html).").unwrap();
        assert_eq!(chip.path, "C:/Users/dev/tmp/ui.html");
        assert_eq!(chip.label.as_ref(), "ui.html");
        assert_eq!(chip.target.as_ref(), "C:/Users/dev/tmp/ui.html");

        let chip = first_link("[src/http.rs:42](/home/dev/app/src/http.rs:42)").unwrap();
        assert_eq!(chip.label.as_ref(), "http.rs · L42");
        assert_eq!(chip.target.as_ref(), "/home/dev/app/src/http.rs:42");

        assert_eq!(
            first_link("<file:///C:/My%20Docs/a.md>")
                .unwrap()
                .label
                .as_ref(),
            "a.md"
        );
    }

    #[test]
    fn other_links_stay_links() {
        assert_eq!(first_link("[the docs](C:/a/readme.md)"), None);
        assert_eq!(first_link("[x.rs](https://github.com/a/x.rs)"), None);
        assert_eq!(first_link("[![shot](C:/a/s.png)](C:/a/s.png)"), None);
    }

    #[test]
    fn inline_code_holding_an_absolute_path_becomes_a_chip() {
        let chip = super::code_path(r"C:\Users\dev\tmp\newsletter.html").unwrap();
        assert_eq!(chip.label.as_ref(), "newsletter.html");
        assert_eq!(
            super::code_path("/home/dev/src/main.rs:12")
                .unwrap()
                .label
                .as_ref(),
            "main.rs · L12"
        );
        assert_eq!(super::code_path("src/main.rs"), None);
        assert_eq!(super::code_path("C:/My Docs/a.md"), None);
        assert_eq!(super::code_path("C:/a/…/b.rs"), None);
        assert_eq!(super::code_path("/home/.../b.rs"), None);
        assert_eq!(super::code_path("npm run dev"), None);
    }
}
