//! The conversation compacting, or compacted, as a divider in the transcript.

use crate::format::compact_number;
use crate::m;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::CompactionStatus;
use hoshi_ui::LoopExt as _;
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::Colors;
use std::time::Duration;

pub use hoshi_core::is_compact_command;

pub fn compaction_label(
    status: CompactionStatus,
    before_tokens: Option<u64>,
    after_tokens: Option<u64>,
    error: Option<&str>,
) -> String {
    let label = match (status, before_tokens, after_tokens) {
        (CompactionStatus::Running, ..) => m::compaction_running(),
        (CompactionStatus::Failed, ..) => m::compaction_failed(),
        (CompactionStatus::Done, Some(before), Some(after)) => {
            m::compaction_done_sizes(compact_number(after), compact_number(before))
        }
        (CompactionStatus::Done, ..) => m::compaction_done(),
    };
    match error {
        Some(error) => format!("{label} · {error}"),
        None => label,
    }
}

pub fn compaction_row(
    id: &str,
    status: CompactionStatus,
    label: String,
    colors: &Colors,
) -> AnyElement {
    let rule = || div().h_px().flex_1().bg(colors.border);
    let running = status == CompactionStatus::Running;
    let icon = Icon::new(if running {
        IconName::LoaderCircle
    } else {
        IconName::Shrink
    })
    .size_3p5()
    .flex_none();
    let icon = if running {
        icon.with_loop(hoshi_ui::looping(Duration::from_secs(1)), |icon, delta| {
            icon.transform(Transformation::rotate(percentage(delta)))
        })
        .into_any_element()
    } else {
        icon.into_any_element()
    };
    let selector = format!("compaction-{id}");
    div()
        .debug_selector(move || selector.clone())
        .flex()
        .items_center()
        .gap_3()
        .text_xs()
        .text_color(if status == CompactionStatus::Failed {
            colors.status_error
        } else {
            colors.muted_foreground
        })
        .child(rule())
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .child(icon)
                .child(div().min_w_0().truncate().child(label)),
        )
        .child(rule())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{compaction_label, is_compact_command};
    use hoshi_core::CompactionStatus;

    #[test]
    fn only_a_bare_compact_command_is_one() {
        assert!(is_compact_command("/compact"));
        assert!(is_compact_command("  /COMPACT keep the plan\n"));
        assert!(!is_compact_command("/compacted"));
        assert!(!is_compact_command("please /compact"));
        assert!(!is_compact_command("/comp"));
        assert!(!is_compact_command("/compáct"));
    }

    #[test]
    fn a_done_compaction_tells_both_sizes_and_a_failure_its_reason() {
        assert_eq!(
            compaction_label(CompactionStatus::Done, Some(152_300), Some(12_000), None),
            "Context compacted · 152.3K → 12K tokens"
        );
        assert_eq!(
            compaction_label(CompactionStatus::Done, None, None, None),
            "Context compacted"
        );
        assert_eq!(
            compaction_label(CompactionStatus::Failed, None, None, Some("too long")),
            "Context compaction failed · too long"
        );
    }
}
