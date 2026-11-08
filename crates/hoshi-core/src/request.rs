use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where a request the agent made mid-turn stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RequestStatus {
    Pending,
    Resolved,
    /// Withdrawn by the agent, or its turn ended before an answer.
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ApprovalDecision {
    Accept,
    /// Accepts this call and the like of it until the session ends.
    AcceptForSession,
    /// The agent goes on without it, told why when there is feedback.
    Decline,
    /// Declines and stops the turn.
    Cancel,
}

impl ApprovalDecision {
    pub fn accepts(self) -> bool {
        matches!(self, Self::Accept | Self::AcceptForSession)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Question {
    /// A short label for the question, such as "Color".
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
    /// Markdown the agent attached to compare options, such as a code sample.
    pub preview: Option<String>,
}

/// The user's answer to a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum RequestResponse {
    /// For a tool approval or a proposed plan.
    Decision {
        decision: ApprovalDecision,
        /// What the agent should do instead; only with `Decline`.
        feedback: Option<String>,
    },
    /// One answer per question, in order: an option's label, several joined with ", ",
    /// or the user's own text.
    Answers { answers: Vec<String> },
}
