//! How each new worktree of a project is prepared: files to copy, a command and whether the
//! agent waits for it, with a way to share the setup as text and take one someone shared.

use crate::m;
use gpui_kit::component::input::TextareaState;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use hoshi_core::{Command, CommandError, Project, WorktreeSetup};
use hoshi_engine::Engine;
use hoshi_ui::prelude::*;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const SHARE_VERSION: u64 = 1;
const COPIED: Duration = Duration::from_millis(1500);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SetupFiles {
    /// Relative, `/`-separated, without blanks or repeats.
    pub files: Vec<String>,
    /// Lines refused as written, trimmed.
    pub invalid: Vec<String>,
}

/// One file per line, as the dialog takes them.
pub fn parse_setup_files<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> SetupFiles {
    let mut parsed = SetupFiles::default();
    for line in lines {
        match hoshi_core::setup_file_path(line.as_ref()) {
            Ok(Some(path)) if !parsed.files.contains(&path) => parsed.files.push(path),
            Ok(_) => {}
            Err(_) => parsed.invalid.push(line.as_ref().trim().to_owned()),
        }
    }
    parsed
}

/// The setup as text to paste into another hoshi.
pub fn share_setup(setup: &WorktreeSetup) -> String {
    json!({
        "hoshiWorktreeSetup": SHARE_VERSION,
        "files": setup.files,
        "command": setup.command,
        "wait": setup.wait,
    })
    .to_string()
}

/// A setup shared with [`share_setup`], or `None` when the text isn't one.
pub fn parse_shared_setup(text: &str) -> Option<WorktreeSetup> {
    let value: Value = serde_json::from_str(text.trim()).ok()?;
    let shared = value.as_object()?;
    if shared.get("hoshiWorktreeSetup")?.as_u64() != Some(SHARE_VERSION) {
        return None;
    }
    let files = match shared.get("files") {
        None => Vec::new(),
        Some(files) => files
            .as_array()?
            .iter()
            .map(|file| file.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?,
    };
    let command = match shared.get("command") {
        None => String::new(),
        Some(command) => command.as_str()?.to_owned(),
    };
    let wait = match shared.get("wait") {
        None => false,
        Some(wait) => wait.as_bool()?,
    };
    Some(WorktreeSetup {
        files,
        command,
        wait,
    })
}

pub enum WorktreeSetupEvent {
    Close,
}

pub struct WorktreeSetupForm {
    project: Project,
    engine: Arc<Engine>,
    files: Entity<TextareaState>,
    command: Entity<TextareaState>,
    wait: bool,
    saving: bool,
    /// A path the engine refused when saving.
    refused: Option<String>,
    /// The import step's text, while it shows.
    import: Option<Entity<TextareaState>>,
    copied: Option<Task<()>>,
    share_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<WorktreeSetupEvent> for WorktreeSetupForm {}

impl WorktreeSetupForm {
    pub fn new(
        project: Project,
        engine: Arc<Engine>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let setup = project.worktree_setup.clone().unwrap_or(WorktreeSetup {
            files: Vec::new(),
            command: String::new(),
            wait: false,
        });
        let files = cx.new(|cx| {
            crate::locale::textarea(m::worktree_setup_files_placeholder, window, cx)
                .auto_grow(3, 8)
                .default_value(setup.files.join("\n"))
        });
        let command = cx.new(|cx| {
            crate::locale::textarea(m::worktree_setup_placeholder, window, cx)
                .auto_grow(3, 8)
                .default_value(setup.command.clone())
        });
        let subscriptions = vec![
            cx.observe(&files, |this, _, cx| {
                this.refused = None;
                cx.notify();
            }),
            cx.observe(&command, |_, _, cx| cx.notify()),
        ];
        files.update(cx, |files, cx| files.focus(window, cx));
        Self {
            project,
            engine,
            files,
            command,
            wait: setup.wait,
            saving: false,
            refused: None,
            import: None,
            copied: None,
            share_open: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn project_title(&self) -> &str {
        &self.project.title
    }

    pub fn importing(&self) -> bool {
        self.import.is_some()
    }

    fn parsed(&self, cx: &App) -> SetupFiles {
        parse_setup_files(self.files.read(cx).value().lines())
    }

    fn invalid(&self, cx: &App) -> Vec<String> {
        let mut invalid = self.parsed(cx).invalid;
        if let Some(refused) = &self.refused
            && !invalid.contains(refused)
        {
            invalid.push(refused.clone());
        }
        invalid
    }

    fn setup(&self, cx: &App) -> Option<WorktreeSetup> {
        let files = self.parsed(cx).files;
        let command = self.command.read(cx).value().trim().to_owned();
        (!files.is_empty() || !command.is_empty()).then_some(WorktreeSetup {
            files,
            command,
            wait: self.wait,
        })
    }

    pub fn can_save(&self, cx: &App) -> bool {
        !self.saving && self.invalid(cx).is_empty()
    }

    pub fn can_import(&self, cx: &App) -> bool {
        self.shared(cx)
            .is_some_and(|shared| parse_setup_files(&shared.files).invalid.is_empty())
    }

    fn shared(&self, cx: &App) -> Option<WorktreeSetup> {
        parse_shared_setup(&self.import.as_ref()?.read(cx).value())
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save(cx) {
            return;
        }
        self.saving = true;
        cx.notify();
        let command = Command::ProjectWorktreeSetupSet {
            project_id: self.project.id.clone(),
            setup: self.setup(cx),
        };
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.dispatch(command) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => cx.emit(WorktreeSetupEvent::Close),
                Err(error) => {
                    if let CommandError::SetupFileInvalid { path } = error {
                        this.refused = Some(path);
                    } else {
                        eprintln!("hoshi: failed to save the worktree setup: {error:?}");
                    }
                    this.saving = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = self.setup(cx) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(share_setup(&setup)));
        self.copied = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED).await;
            this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn start_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = cx.new(|cx| {
            crate::locale::textarea(m::worktree_setup_import_placeholder, window, cx)
                .auto_grow(4, 10)
        });
        self._subscriptions
            .push(cx.observe(&text, |_, _, cx| cx.notify()));
        text.update(cx, |text, cx| text.focus(window, cx));
        self.import = Some(text);
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        self.import = None;
        cx.notify();
    }

    pub fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shared) = self.shared(cx).filter(|_| self.can_import(cx)) else {
            return;
        };
        self.files.update(cx, |files, cx| {
            files.set_value(shared.files.join("\n"), window, cx)
        });
        self.command.update(cx, |command, cx| {
            command.set_value(shared.command.clone(), window, cx)
        });
        self.wait = shared.wait;
        self.refused = None;
        self.import = None;
        cx.notify();
    }

    /// The share menu, first in the dialog's footer.
    pub fn share_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let this = cx.weak_entity();
        let can_copy = self.setup(cx).is_some() && self.invalid(cx).is_empty();
        let copied = self.copied.is_some();
        div().mr_auto().child(
            Popover::new("worktree-setup-share")
                .open(self.share_open)
                .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                    this.share_open = change.open;
                    cx.notify();
                }))
                .trigger(move |_| {
                    Button::new("worktree-setup-share-trigger")
                        .icon(if copied {
                            IconName::Check
                        } else {
                            IconName::Share2
                        })
                        .label(if copied {
                            m::copied()
                        } else {
                            m::worktree_setup_share()
                        })
                        .variant(ButtonVariant::Ghost)
                })
                .w(px(176.))
                .menu(move |menu, _, _| {
                    let copying = this.clone();
                    let importing = this.clone();
                    menu.item(
                        MenuOption::new("worktree-setup-copy", m::worktree_setup_copy())
                            .disabled(!can_copy)
                            .on_select(move |_, cx| {
                                copying.update(cx, |this, cx| this.copy(cx)).ok();
                            }),
                    )
                    .item(
                        MenuOption::new("worktree-setup-import", m::worktree_setup_import_menu())
                            .on_select(move |window, cx| {
                                importing
                                    .update(cx, |this, cx| this.start_import(window, cx))
                                    .ok();
                            }),
                    )
                }),
        )
    }

    fn form(&self, colors: &Colors, cx: &mut Context<Self>) -> Div {
        let invalid = self.invalid(cx);
        let mono = cx.theme().mono_font_family.clone();
        let label = |text: String| {
            div()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(text)
        };
        let hint = label;
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(m::worktree_setup_files()))
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_xs()
                            .child(text_area(&self.files)),
                    )
                    .when(!invalid.is_empty(), |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_col()
                                .text_xs()
                                .text_color(colors.destructive)
                                .children(invalid.iter().map(m::worktree_setup_file_invalid)),
                        )
                    })
                    .child(hint(m::worktree_setup_files_hint())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(m::worktree_setup_command()))
                    .child(
                        div()
                            .font_family(mono)
                            .text_xs()
                            .child(text_area(&self.command)),
                    )
                    .child(hint(m::worktree_setup_command_hint())),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(
                        div().mt(px(2.)).child(
                            Switch::new("worktree-setup-wait")
                                .checked(self.wait)
                                .on_change({
                                    let this = cx.weak_entity();
                                    move |wait, _, cx| {
                                        this.update(cx, |this, cx| {
                                            this.wait = wait;
                                            cx.notify();
                                        })
                                        .ok();
                                    }
                                }),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .text_sm()
                            .child(m::worktree_setup_wait())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(m::worktree_setup_wait_hint()),
                            ),
                    ),
            )
    }

    fn import_step(&self, text: &Entity<TextareaState>, colors: &Colors, cx: &App) -> Div {
        let mono = cx.theme().mono_font_family.clone();
        let written = !text.read(cx).value().trim().is_empty();
        let Some(shared) = self.shared(cx) else {
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.muted_foreground)
                        .child(m::worktree_setup_import_description()),
                )
                .child(div().font_family(mono).text_xs().child(text_area(text)))
                .when(written, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(colors.destructive)
                            .child(m::worktree_setup_import_invalid()),
                    )
                });
        };
        let invalid = parse_setup_files(&shared.files).invalid;
        let block = |label: String, value: String| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(label),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("preview-{value}")))
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .rounded_lg()
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.muted.opacity(0.4))
                        .px(px(10.))
                        .py_2()
                        .text_xs()
                        .map(|this| {
                            if value.is_empty() {
                                this.text_color(colors.muted_foreground)
                                    .child(m::worktree_setup_import_none())
                            } else {
                                this.font_family(mono.clone()).child(value)
                            }
                        }),
                )
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.warning.opacity(0.3))
                    .bg(colors.warning.opacity(0.08))
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .size_4()
                            .flex_none()
                            .mt(px(2.))
                            .text_color(colors.warning),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(colors.warning)
                                    .child(m::worktree_setup_import_warning_title()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(m::worktree_setup_import_warning_body()),
                            ),
                    ),
            )
            .child(block(m::worktree_setup_files(), shared.files.join("\n")))
            .when(!invalid.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .text_xs()
                        .text_color(colors.destructive)
                        .children(invalid.iter().map(m::worktree_setup_file_invalid)),
                )
            })
            .child(block(m::worktree_setup_command(), shared.command.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(m::worktree_setup_import_replaces(&self.project.title)),
            )
    }
}

impl Render for WorktreeSetupForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        match self.import.clone() {
            Some(text) => self.import_step(&text, &colors, cx),
            None => self.form(&colors, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WorktreeSetup, parse_setup_files, parse_shared_setup, share_setup};

    #[test]
    fn files_are_made_relative_and_bad_ones_listed() {
        let parsed = parse_setup_files([
            ".env",
            " ./config\\local.json ",
            "",
            ".env",
            "../out",
            "C:/x",
        ]);
        assert_eq!(parsed.files, [".env", "config/local.json"]);
        assert_eq!(parsed.invalid, ["../out", "C:/x"]);
    }

    #[test]
    fn a_shared_setup_reads_back() {
        let setup = WorktreeSetup {
            files: vec![".env".into()],
            command: "bun install".into(),
            wait: true,
        };
        assert_eq!(parse_shared_setup(&share_setup(&setup)), Some(setup));
    }

    #[test]
    fn other_text_is_not_a_shared_setup() {
        assert_eq!(parse_shared_setup("bun install"), None);
        assert_eq!(parse_shared_setup(r#"{"hoshiWorktreeSetup": 2}"#), None);
        assert_eq!(
            parse_shared_setup(r#"{"hoshiWorktreeSetup": 1, "wait": "yes"}"#),
            None
        );
        assert_eq!(
            parse_shared_setup(r#"{"hoshiWorktreeSetup": 1}"#),
            Some(WorktreeSetup {
                files: Vec::new(),
                command: String::new(),
                wait: false,
            })
        );
    }
}
