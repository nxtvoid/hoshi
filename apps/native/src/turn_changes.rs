//! The card under a finished turn's last answer: the files the turn changed, as folders and
//! files, each opening the turn's diff in the right panel.

use crate::m;
use crate::transcript::TranscriptView;
use crate::work::WorkContext;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use hoshi_core::{DiffFile, ItemContent, ThreadItem};
use hoshi_ui::assets::file_icon;
use hoshi_ui::prelude::*;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// Cards with more files than this start closed.
pub const OPEN_CHANGES_LIMIT: usize = 25;
const CHANGES_SUFFIX: &str = ":changes";

/// A card asks for the right panel's diff of its turn, at `path` when given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenTurnDiff {
    pub turn_id: String,
    pub path: Option<String>,
}

impl EventEmitter<OpenTurnDiff> for TranscriptView {}

/// Each turn's changes by the id of the answer that shows them: its last one. A turn that
/// never answered shows none.
pub fn answers_with_changes(items: &[ThreadItem]) -> HashMap<String, usize> {
    let mut answers = HashMap::new();
    let mut answer: Option<usize> = None;
    for (index, item) in items.iter().enumerate() {
        match &item.content {
            ItemContent::UserMessage { steer: false, .. } => answer = None,
            ItemContent::AssistantMessage { .. } => answer = Some(index),
            ItemContent::TurnChanges { files: Some(files) } if !files.is_empty() => {
                if let Some(answer) = answer {
                    answers.insert(items[answer].id.clone(), index);
                }
            }
            _ => {}
        }
    }
    answers
}

/// Whether a card shows its files: as the user left it, else open unless it has many.
pub fn is_card_open(chosen: Option<bool>, file_count: usize) -> bool {
    chosen.unwrap_or(file_count <= OPEN_CHANGES_LIMIT)
}

pub fn totals(files: &[DiffFile]) -> (u32, u32) {
    files.iter().fold((0, 0), |(additions, deletions), file| {
        (
            additions + file.additions.unwrap_or(0),
            deletions + file.deletions.unwrap_or(0),
        )
    })
}

#[derive(Debug, PartialEq)]
pub enum Node<'a> {
    Folder {
        /// Unique within the tree; a chain of single folders shows as `a/b/c`.
        path: String,
        name: String,
        additions: u32,
        deletions: u32,
        children: Vec<Node<'a>>,
    },
    File {
        name: String,
        file: &'a DiffFile,
    },
}

#[derive(Default)]
struct Draft<'a> {
    folders: Vec<(String, Draft<'a>)>,
    files: Vec<&'a DiffFile>,
}

/// Changed files as folders and files, folders first and each sorted by name. A folder whose
/// only child is a folder shares its row: `src/components/chat`.
pub fn build_tree(files: &[DiffFile]) -> Vec<Node<'_>> {
    let mut root = Draft::default();
    for file in files {
        let path = file.path.replace('\\', "/");
        let parts: Vec<&str> = path.split('/').collect();
        let mut draft = &mut root;
        for part in &parts[..parts.len() - 1] {
            let at = match draft.folders.iter().position(|(name, _)| name == part) {
                Some(at) => at,
                None => {
                    draft.folders.push(((*part).to_owned(), Draft::default()));
                    draft.folders.len() - 1
                }
            };
            draft = &mut draft.folders[at].1;
        }
        draft.files.push(file);
    }
    nodes_of(root, "")
}

fn nodes_of<'a>(draft: Draft<'a>, prefix: &str) -> Vec<Node<'a>> {
    let mut folders = draft.folders;
    folders.sort_by(|(a, _), (b, _)| natural_cmp(a, b));
    let mut files: Vec<Node<'a>> = draft
        .files
        .into_iter()
        .map(|file| {
            let path = file.path.replace('\\', "/");
            let name = path.rsplit('/').next().unwrap_or_default().to_owned();
            Node::File { name, file }
        })
        .collect();
    files.sort_by(|a, b| natural_cmp(a.name(), b.name()));
    folders
        .into_iter()
        .map(|(name, child)| folder_of(name, child, prefix))
        .chain(files)
        .collect()
}

fn folder_of<'a>(name: String, draft: Draft<'a>, prefix: &str) -> Node<'a> {
    let mut label = name;
    let mut current = draft;
    while current.files.is_empty() && current.folders.len() == 1 {
        let (child_name, child) = current.folders.pop().expect("one folder");
        label = format!("{label}/{child_name}");
        current = child;
    }
    let path = format!("{prefix}{label}/");
    let children = nodes_of(current, &path);
    let (additions, deletions) = children.iter().fold((0, 0), |(a, d), node| {
        let (na, nd) = node.changes();
        (a + na, d + nd)
    });
    Node::Folder {
        path,
        name: label,
        additions,
        deletions,
        children,
    }
}

impl Node<'_> {
    fn name(&self) -> &str {
        match self {
            Node::Folder { name, .. } | Node::File { name, .. } => name,
        }
    }

    fn changes(&self) -> (u32, u32) {
        match self {
            Node::Folder {
                additions,
                deletions,
                ..
            } => (*additions, *deletions),
            Node::File { file, .. } => (file.additions.unwrap_or(0), file.deletions.unwrap_or(0)),
        }
    }
}

/// Whether the tree has a folder to open or close.
pub fn has_folders(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| matches!(node, Node::Folder { .. }))
}

/// Names in the order people read them: ignoring case, with numbers by their value.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(digit) = chars.next_if(char::is_ascii_digit) {
                        digits.push(digit);
                    }
                    digits.trim_start_matches('0').to_owned()
                };
                let (x, y) = (number(&mut a), number(&mut b));
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(&y));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// What the user opened and closed of each card this session, by turn id.
#[derive(Default)]
struct Cards {
    open: HashMap<String, bool>,
    /// Turns whose folders all start open.
    all_open: HashSet<String>,
    /// Folders the user turned from how they start, by turn id.
    flipped: HashMap<String, HashSet<String>>,
}

impl Global for Cards {}

fn update_cards(view: &WeakEntity<TranscriptView>, cx: &mut App, change: impl FnOnce(&mut Cards)) {
    change(cx.default_global::<Cards>());
    view.update(cx, |_, cx| cx.notify()).ok();
}

/// `answer`, with the card of what its turn changed under it when it is the turn's last.
pub fn with_card(answer: AnyElement, item: &ThreadItem, ctx: &WorkContext, cx: &App) -> AnyElement {
    let Some(changes) = ctx
        .turn_changes
        .get(&item.id)
        .and_then(|index| ctx.thread.items.get(*index))
    else {
        return answer;
    };
    let ItemContent::TurnChanges { files: Some(files) } = &changes.content else {
        return answer;
    };
    let turn_id = changes
        .id
        .strip_suffix(CHANGES_SUFFIX)
        .unwrap_or(&changes.id);
    div()
        .flex()
        .flex_col()
        .child(answer)
        .child(card(turn_id, files, ctx.view, ctx.colors, cx))
        .into_any_element()
}

fn open_diff(
    view: &WeakEntity<TranscriptView>,
    turn_id: &str,
    path: Option<String>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let (view, turn_id) = (view.clone(), turn_id.to_owned());
    move |_, _, cx| {
        let event = OpenTurnDiff {
            turn_id: turn_id.clone(),
            path: path.clone(),
        };
        view.update(cx, |_, cx| cx.emit(event)).ok();
    }
}

struct Tree<'a> {
    turn_id: &'a str,
    view: &'a WeakEntity<TranscriptView>,
    colors: &'a Colors,
    mono: SharedString,
    all_open: bool,
    flipped: Option<&'a HashSet<String>>,
}

fn card(
    turn_id: &str,
    files: &[DiffFile],
    view: &WeakEntity<TranscriptView>,
    colors: &Colors,
    cx: &App,
) -> AnyElement {
    let cards = cx.try_global::<Cards>();
    let open = is_card_open(
        cards.and_then(|cards| cards.open.get(turn_id).copied()),
        files.len(),
    );
    let all_open = cards.is_some_and(|cards| cards.all_open.contains(turn_id));
    let (additions, deletions) = totals(files);
    let mono = cx.theme().mono_font_family.clone();
    let surface = if cx.theme().mode.is_dark() {
        colors.card
    } else {
        colors.secondary
    };
    let tree = open.then(|| build_tree(files));
    let folders = tree.as_deref().is_some_and(has_folders);
    let id = |part: &str| SharedString::from(format!("turn-changes-{part}-{turn_id}"));

    let toggle = {
        let (view, turn_id) = (view.clone(), turn_id.to_owned());
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            let turn_id = turn_id.clone();
            update_cards(&view, cx, |cards| {
                cards.open.insert(turn_id, !open);
            });
        }
    };
    let toggle_folders = {
        let (view, turn_id) = (view.clone(), turn_id.to_owned());
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            let turn_id = turn_id.clone();
            update_cards(&view, cx, |cards| {
                cards.flipped.remove(&turn_id);
                if !cards.all_open.remove(&turn_id) {
                    cards.all_open.insert(turn_id);
                }
            });
        }
    };
    let muted = colors.muted_foreground;
    let foreground = colors.foreground;
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .py_2()
        .pr_3()
        .pl(px(6.))
        .child(
            div()
                .id(id("toggle"))
                .debug_selector({
                    let selector = format!("turn-changes-toggle-{turn_id}");
                    move || selector
                })
                .group("turn-changes-toggle")
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(6.))
                .rounded_md()
                .px(px(6.))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground)
                .cursor_pointer()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .text_color(muted)
                        .group_hover("turn-changes-toggle", move |style| {
                            style.text_color(foreground)
                        })
                        .child(
                            Icon::new(if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size_3p5(),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(m::turn_changed_files(files.len()))
                        .children(stat(id("total"), additions, deletions, &mono, colors)),
                )
                .press_selects_nothing()
                .on_click(toggle),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .when(open && folders, |this| {
                    this.child(
                        Button::new(id("folders"))
                            .icon(if all_open {
                                IconName::ChevronsDownUp
                            } else {
                                IconName::ChevronsUpDown
                            })
                            .size(ButtonSize::IconXs)
                            .variant(ButtonVariant::Ghost)
                            .text_color(muted)
                            .tooltip(if all_open {
                                m::collapse_all_folders()
                            } else {
                                m::expand_all_folders()
                            })
                            .on_click(toggle_folders),
                    )
                })
                .child(
                    Button::new(id("open-diff"))
                        .icon(IconName::FileDiff)
                        .label(m::open_diff())
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .text_color(muted)
                        .tooltip(m::open_full_diff())
                        .on_click(open_diff(
                            view,
                            turn_id,
                            files.first().map(|file| file.path.clone()),
                        )),
                ),
        );
    let body = tree.map(|tree| {
        let context = Tree {
            turn_id,
            view,
            colors,
            mono: mono.clone(),
            all_open,
            flipped: cards.and_then(|cards| cards.flipped.get(turn_id)),
        };
        let mut rows = Vec::new();
        tree_rows(&tree, 0, folders, &context, &mut rows);
        div()
            .id(id("files"))
            .flex()
            .flex_col()
            .max_h(px(320.))
            .overflow_y_scroll()
            .p_2()
            .children(rows)
    });
    div()
        .debug_selector({
            let selector = format!("turn-changes-{turn_id}");
            move || selector
        })
        .mt_4()
        .flex()
        .flex_col()
        .rounded_lg()
        .border_1()
        .border_color(colors.border)
        .bg(surface)
        .child(header)
        .children(body)
        .into_any_element()
}

fn tree_rows(
    nodes: &[Node],
    depth: usize,
    indent_files: bool,
    tree: &Tree,
    rows: &mut Vec<AnyElement>,
) {
    let colors = tree.colors;
    for node in nodes {
        match node {
            Node::Folder {
                path,
                name,
                additions,
                deletions,
                children,
            } => {
                let flipped = tree.flipped.is_some_and(|flipped| flipped.contains(path));
                let open = tree.all_open != flipped;
                let toggle = {
                    let (view, turn_id, path) =
                        (tree.view.clone(), tree.turn_id.to_owned(), path.clone());
                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        let (turn_id, path) = (turn_id.clone(), path.clone());
                        update_cards(&view, cx, |cards| {
                            let flipped = cards.flipped.entry(turn_id).or_default();
                            if !flipped.remove(&path) {
                                flipped.insert(path);
                            }
                        });
                    }
                };
                rows.push(
                    tree_row(format!("turn-changes-folder-{path}"), depth, colors)
                        .debug_selector({
                            let selector = format!("turn-changes-folder-{path}");
                            move || selector
                        })
                        .child(
                            Icon::new(if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size_3p5()
                            .flex_none()
                            .text_color(colors.muted_foreground),
                        )
                        .child(
                            Icon::new(if open {
                                IconName::FolderOpen
                            } else {
                                IconName::Folder
                            })
                            .size_3p5()
                            .flex_none()
                            .text_color(colors.muted_foreground),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(tree.mono.clone())
                                .text_size(rems(0.6875))
                                .text_color(colors.muted_foreground)
                                .child(name.clone()),
                        )
                        .children(stat(
                            SharedString::from(format!("turn-changes-stat-{path}")),
                            *additions,
                            *deletions,
                            &tree.mono,
                            colors,
                        ))
                        .press_selects_nothing()
                        .on_click(toggle)
                        .into_any_element(),
                );
                if open {
                    tree_rows(children, depth + 1, true, tree, rows);
                }
            }
            Node::File { name, file } => {
                let (icon, color) = file_path_icon(name);
                let foreground = colors.foreground;
                rows.push(
                    tree_row(format!("turn-changes-file-{}", file.path), depth, colors)
                        .debug_selector({
                            let selector = format!("turn-changes-file-{}", file.path);
                            move || selector
                        })
                        .when(indent_files || depth > 0, |row| {
                            row.child(div().size_3p5().flex_none())
                        })
                        .child(
                            svg()
                                .path(file_icon(icon))
                                .size_3p5()
                                .flex_none()
                                .text_color(color(colors)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(tree.mono.clone())
                                .text_xs()
                                .text_color(colors.foreground.opacity(0.85))
                                .group_hover("turn-changes-row", move |style| {
                                    style.text_color(foreground)
                                })
                                .child(name.clone()),
                        )
                        .children(stat(
                            SharedString::from(format!("turn-changes-stat-{}", file.path)),
                            file.additions.unwrap_or(0),
                            file.deletions.unwrap_or(0),
                            &tree.mono,
                            colors,
                        ))
                        .press_selects_nothing()
                        .on_click(open_diff(tree.view, tree.turn_id, Some(file.path.clone())))
                        .into_any_element(),
                );
            }
        }
    }
}

fn tree_row(id: String, depth: usize, colors: &Colors) -> Stateful<Div> {
    let hover = colors.accent.opacity(0.6);
    div()
        .id(SharedString::from(id))
        .group("turn-changes-row")
        .flex()
        .flex_none()
        .h_7()
        .w_full()
        .items_center()
        .gap_2()
        .rounded_md()
        .py_1p5()
        .pr_2()
        .pl(px(8. + depth as f32 * 14.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
}

/// `+12 −3`, with how many lines each is on hover; nothing when it changed no line.
fn stat(
    id: SharedString,
    additions: u32,
    deletions: u32,
    mono: &SharedString,
    colors: &Colors,
) -> Option<AnyElement> {
    if additions == 0 && deletions == 0 {
        return None;
    }
    Some(
        div()
            .id(id)
            .ml_auto()
            .flex()
            .flex_none()
            .gap_1()
            .font_family(mono.clone())
            .text_size(rems(0.6875))
            .child(
                div()
                    .text_color(colors.pr_open)
                    .child(format!("+{additions}")),
            )
            .child(
                div()
                    .text_color(colors.status_error)
                    .child(format!("−{deletions}")),
            )
            .with_tooltip(m::diff_stat_label(additions, deletions))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{Node, answers_with_changes, build_tree, has_folders, is_card_open, totals};
    use hoshi_core::{DiffFile, DiffFileStatus, ItemContent, ThreadItem};

    fn file(path: &str, additions: u32, deletions: u32) -> DiffFile {
        DiffFile {
            path: path.into(),
            old_path: None,
            status: DiffFileStatus::Modified,
            additions: Some(additions),
            deletions: Some(deletions),
        }
    }

    fn item(id: &str, content: ItemContent) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: "t".into(),
            created_at: "2026-11-13T10:00:00Z".into(),
            content,
        }
    }

    fn user(id: &str, steer: bool) -> ThreadItem {
        item(
            id,
            ItemContent::UserMessage {
                text: id.into(),
                attachments: Vec::new(),
                steer,
                task_result: None,
                pull_request_update: None,
                resumed: None,
                rewind: None,
                dropped: false,
                model_switch: None,
            },
        )
    }

    fn answer(id: &str) -> ThreadItem {
        item(id, ItemContent::AssistantMessage { text: id.into() })
    }

    fn changes(turn: &str, files: Option<Vec<DiffFile>>) -> ThreadItem {
        item(
            &format!("{turn}:changes"),
            ItemContent::TurnChanges { files },
        )
    }

    fn shape(nodes: &[Node]) -> Vec<String> {
        nodes
            .iter()
            .flat_map(|node| match node {
                Node::Folder {
                    path,
                    additions,
                    deletions,
                    children,
                    ..
                } => std::iter::once(format!("{path} +{additions} -{deletions}"))
                    .chain(shape(children).into_iter().map(|row| format!("  {row}")))
                    .collect::<Vec<_>>(),
                Node::File { name, .. } => vec![name.clone()],
            })
            .collect()
    }

    #[test]
    fn a_turns_changes_go_on_its_last_answer_and_a_turn_without_one_shows_none() {
        let files = Some(vec![file("a.rs", 1, 0)]);
        let items = [
            user("u1", false),
            answer("a1"),
            user("s1", true),
            answer("a2"),
            changes("u1", files.clone()),
            user("u2", false),
            changes("u2", files.clone()),
            user("u3", false),
            answer("a3"),
            changes("u3", None),
            user("u4", false),
            answer("a4"),
            changes("u4", Some(Vec::new())),
        ];
        let answers = answers_with_changes(&items);
        assert_eq!(answers.len(), 1);
        assert_eq!(answers.get("a2"), Some(&4));
    }

    #[test]
    fn files_become_folders_first_with_single_folder_chains_joined() {
        let files = [
            file("src/components/chat/Row10.tsx", 1, 1),
            file("src/components/chat/Row9.tsx", 2, 0),
            file("README.md", 3, 0),
            file("apps\\native\\main.rs", 0, 4),
            file("apps/native/Cargo.toml", 1, 0),
            file("apps/web.ts", 1, 1),
        ];
        let tree = build_tree(&files);
        assert_eq!(
            shape(&tree),
            [
                "apps/ +2 -5",
                "  apps/native/ +1 -4",
                "    Cargo.toml",
                "    main.rs",
                "  web.ts",
                "src/components/chat/ +3 -1",
                "  Row9.tsx",
                "  Row10.tsx",
                "README.md",
            ]
        );
        assert!(has_folders(&tree));
        assert!(!has_folders(&build_tree(&[file("a.rs", 1, 0)])));
    }

    #[test]
    fn totals_skip_binaries_and_many_files_start_closed() {
        let mut binary = file("logo.png", 0, 0);
        binary.additions = None;
        binary.deletions = None;
        assert_eq!(
            totals(&[file("a.rs", 2, 1), binary, file("b.rs", 3, 4)]),
            (5, 5)
        );
        assert!(is_card_open(None, 25));
        assert!(!is_card_open(None, 26));
        assert!(is_card_open(Some(true), 100));
        assert!(!is_card_open(Some(false), 1));
    }
}

#[cfg(test)]
mod ui_tests {
    use super::OpenTurnDiff;
    use crate::state::ThreadState;
    use crate::transcript::TranscriptView;
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, Modifiers, Point, TestAppContext, VisualTestContext,
        WindowBounds, WindowOptions, px, size,
    };
    use hoshi_core::{DiffFile, DiffFileStatus, ItemContent, ThreadItem};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn item(id: &str, content: ItemContent) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: "t".into(),
            created_at: "2026-11-13T10:00:00Z".into(),
            content,
        }
    }

    fn file(path: &str, additions: u32, deletions: u32) -> DiffFile {
        DiffFile {
            path: path.into(),
            old_path: None,
            status: DiffFileStatus::Modified,
            additions: Some(additions),
            deletions: Some(deletions),
        }
    }

    fn open(
        cx: &mut TestAppContext,
        items: Vec<ThreadItem>,
    ) -> (VisualTestContext, Entity<TranscriptView>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        });
        let (window, view) = cx.update(|cx| {
            let thread = cx.new(|_| ThreadState::new("t", items));
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(500.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|cx| TranscriptView::new(thread, false, cx))
            })
            .expect("test window")
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        redraw(&mut cx);
        (cx, view)
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        for _ in 0..3 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        redraw(cx);
    }

    #[gpui_kit::test]
    fn a_finished_turn_lists_its_files_and_each_opens_the_turns_diff(cx: &mut TestAppContext) {
        let items = vec![
            item(
                "u1",
                ItemContent::UserMessage {
                    text: "Change things".into(),
                    attachments: Vec::new(),
                    steer: false,
                    task_result: None,
                    pull_request_update: None,
                    resumed: None,
                    rewind: None,
                    dropped: false,
                    model_switch: None,
                },
            ),
            item(
                "a1",
                ItemContent::AssistantMessage {
                    text: "Done.".into(),
                },
            ),
            item(
                "u1:changes",
                ItemContent::TurnChanges {
                    files: Some(vec![
                        file("src/app/a.rs", 2, 1),
                        file("src/app/b.rs", 1, 0),
                        file("README.md", 3, 0),
                    ]),
                },
            ),
        ];
        let (mut cx, view) = open(cx, items);
        let opened = Rc::new(RefCell::new(Vec::new()));
        let _events = cx.update({
            let opened = opened.clone();
            move |_, cx| {
                cx.subscribe(&view, move |_, event: &OpenTurnDiff, _| {
                    opened.borrow_mut().push(event.clone())
                })
            }
        });

        assert!(cx.debug_bounds("turn-changes-u1").is_some());
        assert!(cx.debug_bounds("turn-changes-file-README.md").is_some());
        assert!(cx.debug_bounds("turn-changes-file-src/app/a.rs").is_none());

        click(&mut cx, "turn-changes-folder-src/app/");
        click(&mut cx, "turn-changes-file-src/app/a.rs");
        assert_eq!(
            *opened.borrow(),
            [OpenTurnDiff {
                turn_id: "u1".into(),
                path: Some("src/app/a.rs".into()),
            }]
        );

        click(&mut cx, "turn-changes-toggle-u1");
        assert!(cx.debug_bounds("turn-changes-file-README.md").is_none());
        assert!(cx.debug_bounds("turn-changes-u1").is_some());
    }

    #[gpui_kit::test]
    fn revealing_a_call_in_a_folded_turn_opens_the_fold_and_scrolls_to_it(cx: &mut TestAppContext) {
        let mut items = vec![
            item(
                "u1",
                ItemContent::UserMessage {
                    text: "Look around".into(),
                    attachments: Vec::new(),
                    steer: false,
                    task_result: None,
                    pull_request_update: None,
                    resumed: None,
                    rewind: None,
                    dropped: false,
                    model_switch: None,
                },
            ),
            item(
                "s1",
                ItemContent::ToolCall {
                    name: "Agent".into(),
                    input: r#"{"description":"Explore","subagent_type":"Explore"}"#.into(),
                    output: Some("found it".into()),
                    is_error: false,
                    detail: None,
                    output_omitted: false,
                    interrupted: false,
                },
            ),
            item(
                "a1",
                ItemContent::AssistantMessage {
                    text: "Found.".into(),
                },
            ),
        ];
        items.extend((0..40).map(|index| {
            item(
                &format!("m{index}"),
                ItemContent::AssistantMessage {
                    text: format!("Message {index}"),
                },
            )
        }));
        let (mut cx, view) = open(cx, items);
        for _ in 0..5 {
            redraw(&mut cx);
        }
        let before = cx
            .debug_bounds("fold-row-fold:u1")
            .expect("the fold is drawn");
        assert!(cx.debug_bounds("subagent-s1").is_none());

        view.update(&mut cx, |view, cx| view.reveal("s1", cx));
        redraw(&mut cx);
        let after = cx.debug_bounds("fold-row-fold:u1").expect("the fold shows");
        assert!(
            after.top() < px(20.),
            "before: {before:?}, after: {after:?}"
        );
        assert!(cx.debug_bounds("subagent-s1").is_some());
    }
}
