use super::{
    CODEX, COPILOT, addable, after_removal, codex_status_detail, listed, status_detail, status_line,
};
use hoshi_core::{
    ChatGptPlan, CodexAccount, DEFAULT_PROVIDER, ProviderAccount, ProviderAccountStatus,
    ProviderPlan, ProviderSettings, ProviderVersion,
};
use hoshi_ui::prelude::Status;

fn installed(version: &str) -> ProviderVersion {
    ProviderVersion {
        installed: Some(version.into()),
        ..ProviderVersion::default()
    }
}

fn ids(providers: Vec<&'static super::KnownProvider>) -> Vec<&'static str> {
    providers.into_iter().map(|provider| provider.id).collect()
}

#[test]
fn the_menu_offers_only_providers_not_added() {
    let mut settings = ProviderSettings::default();
    assert!(addable(&settings).is_empty());
    assert_eq!(ids(listed(&settings)), [DEFAULT_PROVIDER, COPILOT, CODEX]);

    settings.claude_code.added = false;
    assert_eq!(ids(addable(&settings)), [DEFAULT_PROVIDER]);
    assert_eq!(ids(listed(&settings)), [COPILOT, CODEX]);

    settings.copilot.added = false;
    settings.codex.added = false;
    assert_eq!(ids(addable(&settings)), [DEFAULT_PROVIDER, COPILOT, CODEX]);
    assert!(listed(&settings).is_empty());
}

#[test]
fn removing_selects_the_next_provider_else_the_one_before() {
    let both = [DEFAULT_PROVIDER, COPILOT];
    assert_eq!(after_removal(&both, DEFAULT_PROVIDER), Some(COPILOT));
    assert_eq!(after_removal(&both, COPILOT), Some(DEFAULT_PROVIDER));
    assert_eq!(after_removal(&[COPILOT], COPILOT), None);
}

#[test]
fn a_ready_provider_shows_its_version_and_that_it_is_signed_in() {
    let claude = installed("2.1.325");
    let copilot = installed("1.0.112");
    assert_eq!(
        status_line(
            DEFAULT_PROVIDER,
            true,
            Some(ProviderAccountStatus::Ready),
            Some(&claude)
        ),
        ("v2.1.325 · Signed in".to_owned(), None)
    );
    assert_eq!(
        status_line(
            COPILOT,
            true,
            Some(ProviderAccountStatus::Ready),
            Some(&copilot)
        ),
        ("v1.0.112 · Signed in".to_owned(), None)
    );
}

#[test]
fn a_problem_shows_a_dot_and_no_version() {
    let claude = installed("2.1.325");
    assert_eq!(
        status_line(
            COPILOT,
            true,
            Some(ProviderAccountStatus::NotInstalled),
            None
        ),
        ("Not found".to_owned(), Some(Status::Error))
    );
    assert_eq!(
        status_line(
            DEFAULT_PROVIDER,
            true,
            Some(ProviderAccountStatus::SignedOut),
            Some(&claude)
        ),
        ("Not signed in".to_owned(), Some(Status::Waiting))
    );
}

#[test]
fn a_disabled_provider_is_off_without_a_version() {
    let claude = installed("2.1.325");
    assert_eq!(
        status_line(
            DEFAULT_PROVIDER,
            false,
            Some(ProviderAccountStatus::Ready),
            Some(&claude)
        ),
        ("Off".to_owned(), None)
    );
    assert_eq!(
        status_detail(COPILOT, false, None, None, None, "copilot"),
        ("Off".to_owned(), None)
    );
}

#[test]
fn an_unchecked_provider_is_checking() {
    assert_eq!(
        status_line(COPILOT, true, None, None),
        ("Checking…".to_owned(), None)
    );
}

#[test]
fn the_detail_names_the_plan_and_what_to_do_about_a_problem() {
    let account = ProviderAccount {
        email: Some("dev@example.com".into()),
        plan: Some(ProviderPlan::Max),
    };
    assert_eq!(
        status_detail(
            DEFAULT_PROVIDER,
            true,
            Some(ProviderAccountStatus::Ready),
            Some(&account),
            None,
            "claude"
        ),
        ("Signed in · Max".to_owned(), None)
    );
    assert_eq!(
        status_detail(
            COPILOT,
            true,
            Some(ProviderAccountStatus::Ready),
            None,
            Some("nxtvoid"),
            "copilot"
        ),
        ("Signed in as nxtvoid".to_owned(), None)
    );
    assert_eq!(
        status_detail(
            COPILOT,
            true,
            Some(ProviderAccountStatus::SignedOut),
            None,
            None,
            "copilot"
        ),
        (
            "Not signed in · run copilot login in a terminal".to_owned(),
            Some(Status::Waiting)
        )
    );
    assert_eq!(
        status_detail(
            COPILOT,
            true,
            Some(ProviderAccountStatus::NotInstalled),
            None,
            None,
            "copilot"
        ),
        (
            "Not found · hoshi can't start copilot".to_owned(),
            Some(Status::Error)
        )
    );
    assert_eq!(
        status_detail(
            DEFAULT_PROVIDER,
            true,
            Some(ProviderAccountStatus::SignedOut),
            None,
            None,
            "claude"
        ),
        (
            "Not signed in · run claude auth login in a terminal".to_owned(),
            Some(Status::Waiting)
        )
    );
}

#[test]
fn codex_lists_as_signed_in_or_not_installed() {
    let codex = installed("0.169.0");
    assert_eq!(
        status_line(
            CODEX,
            true,
            Some(ProviderAccountStatus::Ready),
            Some(&codex)
        ),
        ("v0.169.0 · Signed in".to_owned(), None)
    );
    assert_eq!(
        status_line(CODEX, true, Some(ProviderAccountStatus::NotInstalled), None),
        ("Not installed".to_owned(), Some(Status::Waiting))
    );
    assert_eq!(
        status_line(
            CODEX,
            true,
            Some(ProviderAccountStatus::SignedOut),
            Some(&codex)
        ),
        ("Not signed in".to_owned(), Some(Status::Waiting))
    );
}

#[test]
fn codex_names_its_chatgpt_account_and_plan() {
    let ready = Some(ProviderAccountStatus::Ready);
    let account = CodexAccount {
        chatgpt: true,
        email: Some("dev@example.com".into()),
        plan: Some(ChatGptPlan::Plus),
    };
    assert_eq!(
        codex_status_detail(true, ready, Some(&account), None),
        ("Signed in as dev@example.com · Plus".to_owned(), None)
    );
    let unnamed = CodexAccount {
        email: None,
        ..account
    };
    assert_eq!(
        codex_status_detail(true, ready, Some(&unnamed), None),
        ("Signed in · Plus".to_owned(), None)
    );
    let api_key = CodexAccount::default();
    assert_eq!(
        codex_status_detail(true, ready, Some(&api_key), None),
        ("Signed in with an API key".to_owned(), None)
    );
    assert_eq!(
        codex_status_detail(true, ready, None, None),
        ("Signed in".to_owned(), None)
    );
}

#[test]
fn a_missing_codex_is_offered_unless_the_settings_name_a_binary() {
    let missing = Some(ProviderAccountStatus::NotInstalled);
    assert_eq!(
        codex_status_detail(true, missing, None, None),
        ("Not installed".to_owned(), Some(Status::Waiting))
    );
    assert_eq!(
        codex_status_detail(true, missing, None, Some("/opt/codex")),
        (
            "Not found · hoshi can't start /opt/codex".to_owned(),
            Some(Status::Error)
        )
    );
    assert_eq!(
        codex_status_detail(true, Some(ProviderAccountStatus::SignedOut), None, None),
        ("Not signed in".to_owned(), Some(Status::Waiting))
    );
    assert_eq!(
        codex_status_detail(false, missing, None, None),
        ("Off".to_owned(), None)
    );
    assert_eq!(
        codex_status_detail(true, None, None, None),
        ("Checking…".to_owned(), None)
    );
}
