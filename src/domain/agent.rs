//! The pr-agent tools and the content of their result files.
//!
//! The types are plain data, so `bus` can carry them between `app` and
//! the runtime. `agent` runs pr-agent and reads and writes the files.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The version of the file format. A reader refuses other versions.
pub(crate) const VERSION: u32 = 1;

/// A pr-agent tool, with its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tool {
    Review,
    Improve,
    Describe,
    /// A question about the pull request.
    Ask(String),
}

/// A pr-agent tool without its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ToolKind {
    Review,
    Improve,
    Describe,
    Ask,
}

impl ToolKind {
    /// Every tool, in display order.
    pub(crate) const ALL: [ToolKind; 4] = [
        ToolKind::Review,
        ToolKind::Improve,
        ToolKind::Describe,
        ToolKind::Ask,
    ];

    /// The pr-agent command, also the name of the result file.
    pub(crate) fn name(self) -> &'static str {
        match self {
            ToolKind::Review => "review",
            ToolKind::Improve => "improve",
            ToolKind::Describe => "describe",
            ToolKind::Ask => "ask",
        }
    }

    /// The pr-agent settings section of the tool.
    pub(crate) fn section(self) -> &'static str {
        match self {
            ToolKind::Review => "pr_reviewer",
            ToolKind::Improve => "pr_code_suggestions",
            ToolKind::Describe => "pr_description",
            ToolKind::Ask => "pr_questions",
        }
    }
}

impl Tool {
    /// The tool without its input.
    pub(crate) fn kind(&self) -> ToolKind {
        match self {
            Tool::Review => ToolKind::Review,
            Tool::Improve => ToolKind::Improve,
            Tool::Describe => ToolKind::Describe,
            Tool::Ask(_) => ToolKind::Ask,
        }
    }
}

/// The content of one result file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    pub version: u32,
    /// The tool name, for example `review`.
    pub tool: String,
    /// The number of the pull request.
    pub pr: u64,
    /// The head commit of the pull request when the run started.
    pub head_sha: String,
    /// The `LiteLLM` name of the model.
    pub model: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    /// The token counts that pr-agent gives, or an empty object.
    pub usage: serde_json::Value,
    /// The JSON of pr-agent. For `ask`, a list of `AskEntry`, newest
    /// first.
    pub data: serde_json::Value,
}

/// One question and its answer in `ask.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AskEntry {
    pub question: String,
    pub answer: String,
    pub head_sha: String,
    pub at: DateTime<Utc>,
}

impl Record {
    /// The `total_tokens` of `usage`. `None` when pr-agent gave no count.
    pub(crate) fn total_tokens(&self) -> Option<u64> {
        self.usage.get("total_tokens")?.as_u64()
    }

    /// The questions and answers of an `ask` record, newest first. Empty
    /// for other tools.
    pub(crate) fn ask_entries(&self) -> Vec<AskEntry> {
        serde_json::from_value(self.data.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_and_sections() {
        let names: Vec<_> = ToolKind::ALL.iter().map(|kind| kind.name()).collect();
        assert_eq!(names, ["review", "improve", "describe", "ask"]);
        assert_eq!(Tool::Ask("why?".into()).kind(), ToolKind::Ask);
        assert_eq!(ToolKind::Improve.section(), "pr_code_suggestions");
    }

    #[test]
    fn record_round_trips() {
        let record = Record {
            version: VERSION,
            tool: "ask".into(),
            pr: 7,
            head_sha: "3a1f2c0".into(),
            model: "m".into(),
            started_at: crate::domain::test_time(),
            finished_at: crate::domain::test_time(),
            usage: serde_json::json!({}),
            data: serde_json::json!([{
                "question": "Why?",
                "answer": "Because.",
                "head_sha": "3a1f2c0",
                "at": "2026-09-21T14:13:20Z"
            }]),
        };
        crate::domain::assert_round_trip(&record);
        assert_eq!(record.total_tokens(), None);
        assert_eq!(record.ask_entries()[0].answer, "Because.");
    }
}
