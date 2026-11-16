use super::SettingsView;
use crate::m;
use gpui_kit::component::input::Input;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{CommandError, ForgeAccount, ForgeKind};
use hoshi_ui::prelude::*;

const BITBUCKET_TOKENS_URL: &str = "https://id.atlassian.com/manage-profile/security/api-tokens";

const BITBUCKET_SCOPES: [&str; 5] = [
    "read:user:bitbucket",
    "read:repository:bitbucket",
    "read:pullrequest:bitbucket",
    "write:pullrequest:bitbucket",
    "read:workspace:bitbucket",
];

fn forge_error_message(error: &CommandError) -> String {
    match error {
        CommandError::ForgeUnauthorized => m::forge_error_unauthorized(),
        CommandError::ForgeRateLimited => m::forge_error_rate_limited(),
        CommandError::ForgeRejected { detail } => detail.clone(),
        CommandError::ForgeUnreachable { .. } => m::forge_error_unreachable(),
        _ => m::forge_error_unavailable(),
    }
}

fn connect_error_message(error: &CommandError) -> String {
    match error {
        CommandError::ForgeUnauthorized => m::bitbucket_error_credentials(),
        CommandError::ForgeScopesMissing { scopes } => m::bitbucket_error_scopes(scopes.join(", ")),
        _ => forge_error_message(error),
    }
}

impl SettingsView {
    fn login(&self, forge: ForgeKind) -> Option<String> {
        self.accounts
            .as_ref()?
            .iter()
            .find(|account| account.forge == forge)?
            .login
            .clone()
    }

    pub(super) fn forges(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.hoshi().clone();
        let checking = self.accounts.is_none();
        let status = |text: String| {
            div()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(text)
        };
        let github = match (checking, self.login(ForgeKind::Github)) {
            (true, _) => m::forge_checking(),
            (false, Some(login)) => m::forge_signed_in(login),
            (false, None) => m::forge_signed_out(),
        };
        let bitbucket_login = self.login(ForgeKind::Bitbucket);
        let form = &self.bitbucket;

        let control = if checking {
            status(m::forge_checking()).into_any_element()
        } else if let Some(login) = bitbucket_login.clone() {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(status(m::forge_signed_in(login)))
                .child(
                    Button::new("bitbucket-disconnect")
                        .label(m::forge_disconnect())
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .disabled(form.pending)
                        .on_click(cx.listener(|this, _, _, cx| this.disconnect_bitbucket(cx))),
                )
                .into_any_element()
        } else if form.open {
            div().into_any_element()
        } else {
            Button::new("bitbucket-connect")
                .label(m::forge_connect())
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Sm)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.bitbucket.open = true;
                    this.bitbucket
                        .email
                        .update(cx, |email, cx| email.focus(window, cx));
                    cx.notify();
                }))
                .into_any_element()
        };

        let bitbucket = SettingsRow::new(m::bitbucket())
            .description(m::bitbucket_description())
            .control(control)
            .when(form.open && bitbucket_login.is_none(), |row| {
                row.child(self.bitbucket_form(&colors, cx))
            });

        vec![
            SettingsGroup::new()
                .title(m::settings_forges())
                .child(
                    SettingsRow::new(m::github())
                        .description(m::github_description())
                        .control(status(github)),
                )
                .child(bitbucket)
                .into_any_element(),
        ]
    }

    fn bitbucket_form(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let form = &self.bitbucket;
        let ready = !form.email.read(cx).value().trim().is_empty()
            && !form.token.read(cx).value().trim().is_empty();
        let mono = cx.theme().mono_font_family.clone();
        let label = |text: String| div().text_xs().font_weight(FontWeight::MEDIUM).child(text);
        let foreground = colors.foreground;
        div()
            .mt_3()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(m::bitbucket_token_help())
                    .child(
                        div()
                            .id("bitbucket-create-token")
                            .text_color(foreground)
                            .underline()
                            .cursor_pointer()
                            .on_click(|_, _, cx| cx.open_url(BITBUCKET_TOKENS_URL))
                            .child(m::bitbucket_create_token()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(BITBUCKET_SCOPES.map(|scope| {
                        div()
                            .px_1p5()
                            .py(px(2.))
                            .rounded_sm()
                            .bg(colors.muted)
                            .text_color(colors.muted_foreground)
                            .text_size(rems(0.6875))
                            .font_family(mono.clone())
                            .child(scope)
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(m::bitbucket_email()))
                    .child(Input::new(&form.email)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(m::bitbucket_api_token()))
                    .child(Input::new(&form.token)),
            )
            .when_some(form.error.clone(), |this, error| {
                this.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_1p5()
                        .text_xs()
                        .text_color(colors.destructive)
                        .child(Icon::new(IconName::CircleAlert).size_3p5().flex_none())
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("bitbucket-cancel")
                            .label(m::cancel())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_bitbucket_form(window, cx)
                            })),
                    )
                    .child(
                        Button::new("bitbucket-submit")
                            .label(if form.pending {
                                m::forge_checking()
                            } else {
                                m::forge_connect()
                            })
                            .size(ButtonSize::Sm)
                            .disabled(form.pending || !ready)
                            .on_click(cx.listener(|this, _, _, cx| this.connect_bitbucket(cx))),
                    ),
            )
    }

    fn close_bitbucket_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.bitbucket.open = false;
        self.bitbucket.error = None;
        self.bitbucket
            .token
            .update(cx, |token, cx| token.set_value("", window, cx));
        cx.notify();
    }

    pub(super) fn load_accounts(&mut self, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let accounts = cx
                .background_spawn(async move { engine.forge_accounts() })
                .await;
            this.update(cx, |this, cx| {
                this.accounts = Some(accounts);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Takes in a forge's new account, here and in the workspace.
    fn account_changed(&mut self, account: ForgeAccount, cx: &mut Context<Self>) {
        let accounts = self.accounts.get_or_insert_with(Vec::new);
        accounts.retain(|each| each.forge != account.forge);
        accounts.push(account.clone());
        self.workspace.update(cx, |workspace, cx| {
            match account.login {
                Some(login) => {
                    workspace.logins.insert(account.forge, login);
                }
                None => {
                    workspace.logins.remove(&account.forge);
                }
            }
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn connect_bitbucket(&mut self, cx: &mut Context<Self>) {
        let email = self.bitbucket.email.read(cx).value().trim().to_string();
        let token = self.bitbucket.token.read(cx).value().trim().to_string();
        if self.bitbucket.pending || email.is_empty() || token.is_empty() {
            return;
        }
        self.bitbucket.pending = true;
        self.bitbucket.error = None;
        cx.notify();
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let connected = cx
                .background_spawn(async move { engine.connect_bitbucket(&email, &token) })
                .await;
            this.update(cx, |this, cx| {
                this.bitbucket.pending = false;
                match connected {
                    Ok(account) => {
                        this.bitbucket.open = false;
                        this.account_changed(account, cx);
                    }
                    Err(error) => this.bitbucket.error = Some(connect_error_message(&error)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn disconnect_bitbucket(&mut self, cx: &mut Context<Self>) {
        self.bitbucket.pending = true;
        cx.notify();
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let disconnected = cx
                .background_spawn(async move { engine.disconnect_forge(ForgeKind::Bitbucket) })
                .await;
            this.update(cx, |this, cx| {
                this.bitbucket.pending = false;
                match disconnected {
                    Ok(()) => this.account_changed(
                        ForgeAccount {
                            forge: ForgeKind::Bitbucket,
                            login: None,
                        },
                        cx,
                    ),
                    Err(error) => this.bitbucket.error = Some(forge_error_message(&error)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
