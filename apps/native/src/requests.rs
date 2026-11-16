//! Answering what the agent asks mid-turn: where the user is in a set of questions.

use hoshi_core::{ApprovalDecision, Question};

/// Where the user is in a set of questions, and what they answered so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestionProgress {
    pub index: usize,
    pub answers: Vec<String>,
    /// The options picked on the current question.
    pub selected: Vec<String>,
}

/// Picks or unpicks an option; a single-choice question keeps only the latest.
pub fn toggle_option(
    progress: &QuestionProgress,
    label: &str,
    multi_select: bool,
) -> QuestionProgress {
    let mut next = progress.clone();
    if !multi_select {
        next.selected = vec![label.to_owned()];
    } else if let Some(at) = next.selected.iter().position(|picked| picked == label) {
        next.selected.remove(at);
    } else {
        next.selected.push(label.to_owned());
    }
    next
}

/// The user's own text wins over the picked options, which join with ", ".
pub fn answer_text(selected: &[String], custom: &str) -> String {
    let text = custom.trim();
    if text.is_empty() {
        selected.join(", ")
    } else {
        text.to_owned()
    }
}

/// Records the answer to the current question and moves to the next. Returns every answer
/// once the last question is answered; an empty answer changes nothing.
pub fn answer_question(
    progress: &QuestionProgress,
    total: usize,
    answer: &str,
) -> (QuestionProgress, Option<Vec<String>>) {
    if answer.trim().is_empty() {
        return (progress.clone(), None);
    }
    let mut answers: Vec<String> = progress
        .answers
        .iter()
        .take(progress.index)
        .cloned()
        .collect();
    answers.push(answer.to_owned());
    if answers.len() >= total {
        return (progress.clone(), Some(answers));
    }
    let next = QuestionProgress {
        index: progress.index + 1,
        answers,
        selected: Vec::new(),
    };
    (next, None)
}

/// Back to the previous question, with what was picked there.
pub fn previous_question(progress: &QuestionProgress, questions: &[Question]) -> QuestionProgress {
    if progress.index == 0 {
        return progress.clone();
    }
    let index = progress.index - 1;
    let answer = progress.answers.get(index).cloned().unwrap_or_default();
    let selected = questions
        .get(index)
        .map(|question| {
            answer
                .split(", ")
                .filter(|part| question.options.iter().any(|option| option.label == *part))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    QuestionProgress {
        index,
        answers: progress.answers.iter().take(index).cloned().collect(),
        selected,
    }
}

/// The answers an approval offers, in their order on its panel. An MCP server's own question,
/// which holds back no call, can be cancelled too; `for_session` when it may be accepted for
/// the session.
pub fn approval_choices(holds_back_a_call: bool, for_session: bool) -> Vec<ApprovalDecision> {
    let mut choices = Vec::with_capacity(4);
    if !holds_back_a_call {
        choices.push(ApprovalDecision::Cancel);
    }
    choices.push(ApprovalDecision::Decline);
    if for_session {
        choices.push(ApprovalDecision::AcceptForSession);
    }
    choices.push(ApprovalDecision::Accept);
    choices
}

#[cfg(test)]
mod tests {
    use super::{
        QuestionProgress, answer_question, answer_text, approval_choices, previous_question,
        toggle_option,
    };
    use hoshi_core::{ApprovalDecision, Question, QuestionOption};

    #[test]
    fn only_an_mcp_servers_own_question_can_be_cancelled() {
        use ApprovalDecision::{Accept, AcceptForSession, Cancel, Decline};
        assert_eq!(
            approval_choices(false, true),
            [Cancel, Decline, AcceptForSession, Accept]
        );
        assert_eq!(approval_choices(false, false), [Cancel, Decline, Accept]);
        assert_eq!(
            approval_choices(true, true),
            [Decline, AcceptForSession, Accept]
        );
    }

    fn question(labels: &[&str]) -> Question {
        Question {
            header: "H".into(),
            question: "Q?".into(),
            options: labels
                .iter()
                .map(|label| QuestionOption {
                    label: (*label).into(),
                    description: String::new(),
                    preview: None,
                })
                .collect(),
            multi_select: true,
        }
    }

    #[test]
    fn a_single_choice_keeps_the_latest_and_a_multi_choice_toggles() {
        let start = QuestionProgress::default();
        let one = toggle_option(&toggle_option(&start, "a", false), "b", false);
        assert_eq!(one.selected, vec!["b"]);
        let many = toggle_option(&toggle_option(&start, "a", true), "b", true);
        assert_eq!(many.selected, vec!["a", "b"]);
        assert_eq!(toggle_option(&many, "a", true).selected, vec!["b"]);
    }

    #[test]
    fn typed_text_wins_over_picked_options() {
        let picked = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(answer_text(&picked, "  "), "a, b");
        assert_eq!(answer_text(&picked, " mine "), "mine");
    }

    #[test]
    fn the_last_answer_returns_them_all_and_an_empty_one_changes_nothing() {
        let start = QuestionProgress::default();
        assert_eq!(answer_question(&start, 2, " "), (start.clone(), None));
        let (second, done) = answer_question(&start, 2, "x");
        assert_eq!((second.index, done), (1, None));
        let (_, done) = answer_question(&second, 2, "y");
        assert_eq!(done, Some(vec!["x".to_owned(), "y".to_owned()]));
    }

    #[test]
    fn going_back_restores_what_was_picked_there() {
        let questions = vec![question(&["a", "b", "c"]), question(&["d"])];
        let at_second = QuestionProgress {
            index: 1,
            answers: vec!["a, c".into()],
            selected: Vec::new(),
        };
        let back = previous_question(&at_second, &questions);
        assert_eq!(back.index, 0);
        assert_eq!(back.selected, vec!["a", "c"]);
        assert!(back.answers.is_empty());
    }
}
