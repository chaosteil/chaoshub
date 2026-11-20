//! The agent backends, their tools and the content of their result
//! files.
//!
//! The types are plain data, so `bus` can carry them between `app` and
//! the runtime. `agent` runs the backend and reads and writes the files.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The version of the file format. A reader refuses other versions.
pub(crate) const VERSION: u32 = 1;

/// The program that runs the agent tools: the `[agent] backend` key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Backend {
    /// Alibaba Open Code Review, the `ocr` CLI. It has only `review`.
    #[default]
    OpenCodeReview,
    /// qodo pr-agent. It has every tool.
    PrAgent,
}

impl Backend {
    /// The short name in the UI texts: `ocr` or `pr-agent`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Backend::OpenCodeReview => "ocr",
            Backend::PrAgent => "pr-agent",
        }
    }

    /// True when the backend has the tool `kind`.
    pub(crate) fn supports(self, kind: ToolKind) -> bool {
        match self {
            Backend::OpenCodeReview => kind == ToolKind::Review,
            Backend::PrAgent => true,
        }
    }

    /// The backend of a file from before the `backend` field.
    fn of_old_files() -> Self {
        Backend::PrAgent
    }
}

/// A tool of an agent backend, with its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tool {
    Review,
    Improve,
    Describe,
    /// A question about the pull request.
    Ask(String),
}

/// A tool of an agent backend, without its input.
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
    /// The backend that made the result. A file from before the field
    /// has `pr_agent`.
    #[serde(default = "Backend::of_old_files")]
    pub backend: Backend,
    /// The tool name, for example `review`.
    pub tool: String,
    /// The number of the pull request.
    pub pr: u64,
    /// The head commit of the pull request when the run started.
    pub head_sha: String,
    /// The name of the model. pr-agent gives the `LiteLLM` name. ocr
    /// gives `<provider>/<model>`, or only the model.
    pub model: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    /// The token counts that the backend gives, or an empty object.
    pub usage: serde_json::Value,
    /// The JSON of pr-agent. For `ask`, a list of `AskEntry`, newest
    /// first. For ocr, the review in the form of pr-agent (see
    /// `ocr_review`).
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
    /// The `total_tokens` of `usage`. `None` when the backend gave no
    /// count.
    pub(crate) fn total_tokens(&self) -> Option<u64> {
        self.usage.get("total_tokens")?.as_u64()
    }

    /// The questions and answers of an `ask` record, newest first. Empty
    /// for other tools.
    pub(crate) fn ask_entries(&self) -> Vec<AskEntry> {
        serde_json::from_value(self.data.clone()).unwrap_or_default()
    }
}

/// The results of one pull request: the record of each tool that has a
/// file, and the error of the last failed run or read of each tool.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AgentResults {
    pub records: BTreeMap<ToolKind, Record>,
    /// A successful run of the tool clears its error.
    pub errors: BTreeMap<ToolKind, String>,
}

/// One improve suggestion of pr-agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
#[allow(
    clippy::struct_field_names,
    reason = "the field names are the JSON keys of pr-agent"
)]
pub(crate) struct Suggestion {
    pub relevant_file: String,
    pub relevant_lines_start: u64,
    pub relevant_lines_end: u64,
    pub existing_code: String,
    pub improved_code: String,
    pub suggestion_content: String,
    pub one_sentence_summary: String,
    pub label: String,
    pub score: Option<serde_json::Value>,
}

/// The improve suggestions in the JSON of pr-agent. A suggestion that
/// does not parse is left out.
pub(crate) fn suggestions(data: &serde_json::Value) -> Vec<Suggestion> {
    data.get("code_suggestions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| serde_json::from_value(item.clone()).ok())
        .collect()
}

/// A token count in a short form: `950`, `12k`.
pub(crate) fn short_tokens(tokens: u64) -> String {
    if tokens < 1000 {
        tokens.to_string()
    } else {
        format!("{}k", (tokens + 500) / 1000)
    }
}

/// A JSON value as text: a string as it is, other values as JSON.
fn text_of(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.trim().to_owned(),
        other => other.to_string(),
    }
}

/// The label of a field of the review: `estimated_effort_to_review_[1-5]`
/// gives `Estimated effort to review`.
fn field_label(key: &str) -> String {
    let words = key
        .trim_end_matches("_[1-5]")
        .replace('_', " ")
        .trim()
        .to_owned();
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The markdown of a review, made from the JSON fields of pr-agent:
/// one line per field, then the key issues.
pub(crate) fn review_markdown(data: &serde_json::Value) -> String {
    let Some(review) = data.get("review").and_then(serde_json::Value::as_object) else {
        return String::new();
    };
    let mut text = String::new();
    for (key, value) in review {
        if key == "key_issues_to_review" || value.is_array() || value.is_object() {
            continue;
        }
        let _ = writeln!(text, "- **{}:** {}", field_label(key), text_of(value));
    }
    let issues = review
        .get("key_issues_to_review")
        .and_then(serde_json::Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if !issues.is_empty() {
        text.push_str("\n**Key issues**\n\n");
        for issue in issues {
            let field = |name: &str| issue.get(name).map(text_of).unwrap_or_default();
            let lines = match (issue.get("start_line"), issue.get("end_line")) {
                (Some(start), Some(end)) if start != end => format!(":{start}-{end}"),
                (Some(start), _) => format!(":{start}"),
                _ => String::new(),
            };
            let _ = writeln!(
                text,
                "- **{}** `{}{lines}`: {}",
                field("issue_header"),
                field("relevant_file"),
                field("issue_content")
            );
        }
    }
    text.trim().to_owned()
}

/// The rank of an ocr severity: `critical` first. An unknown severity
/// comes last.
fn severity_rank(severity: &str) -> u8 {
    match severity {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

/// The review of the JSON output of `ocr review --format json`, in the
/// form of pr-agent, so that the Agent pane and codeview read it as a
/// pr-agent review. Each comment of ocr becomes a key issue, the most
/// severe first:
///
/// - `issue_header`: the severity and the category, for example
///   `High bug`.
/// - `relevant_file`, `issue_content`: the `path` and the `content`.
/// - `start_line`, `end_line`: only when ocr found the lines. ocr gives
///   0 and 0 when it did not.
/// - `severity`, `category`, `existing_code`, `suggestion_code`: as ocr
///   gives them.
///
/// The review also gets `files_reviewed`, the `message` of ocr as
/// `summary`, the `status` when it is not a success, and the number of
/// `warnings`.
pub(crate) fn ocr_review(output: &serde_json::Value) -> serde_json::Value {
    use serde_json::{Map, Value};
    let text = |value: &Value, name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_owned()
    };
    let mut comments: Vec<&Value> = output
        .get("comments")
        .and_then(Value::as_array)
        .map(|comments| comments.iter().filter(|c| c.is_object()).collect())
        .unwrap_or_default();
    comments.sort_by_key(|comment| severity_rank(&text(comment, "severity")));
    let issues: Vec<Value> = comments
        .into_iter()
        .map(|comment| {
            let severity = text(comment, "severity");
            let category = text(comment, "category");
            let header = match (severity.as_str(), category.as_str()) {
                ("", "") => "Issue".to_owned(),
                ("", category) => field_label(category),
                (severity, "") => field_label(severity),
                (severity, category) => format!("{} {category}", field_label(severity)),
            };
            let mut issue = Map::new();
            issue.insert("relevant_file".into(), text(comment, "path").into());
            issue.insert("issue_header".into(), header.into());
            issue.insert("issue_content".into(), text(comment, "content").into());
            let line = |name: &str| comment.get(name).and_then(Value::as_u64).unwrap_or(0);
            let (start, end) = (line("start_line"), line("end_line"));
            if start > 0 {
                issue.insert("start_line".into(), start.into());
                issue.insert("end_line".into(), end.max(start).into());
            }
            for name in ["severity", "category", "existing_code", "suggestion_code"] {
                let value = text(comment, name);
                if !value.is_empty() {
                    issue.insert(name.into(), value.into());
                }
            }
            Value::Object(issue)
        })
        .collect();
    let mut review = Map::new();
    if let Some(files) = output.pointer("/summary/files_reviewed") {
        review.insert("files_reviewed".into(), files.clone());
    }
    let message = text(output, "message");
    if !message.is_empty() {
        review.insert("summary".into(), message.into());
    }
    let status = text(output, "status");
    if !matches!(status.as_str(), "" | "success" | "complete") {
        review.insert("status".into(), status.into());
    }
    let warnings = output
        .get("warnings")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if warnings > 0 {
        review.insert("warnings".into(), warnings.into());
    }
    review.insert("key_issues_to_review".into(), issues.into());
    serde_json::json!({ "review": review })
}

/// The new title of a describe result. `None` when it has none.
pub(crate) fn describe_title(data: &serde_json::Value) -> Option<String> {
    data.get("title")
        .map(text_of)
        .filter(|title| !title.is_empty())
}

/// The new body of a describe result: the `description`, then a
/// walkthrough with one line per entry of `pr_files`.
pub(crate) fn describe_body(data: &serde_json::Value) -> String {
    let mut body = data.get("description").map(text_of).unwrap_or_default();
    let files = data
        .get("pr_files")
        .and_then(serde_json::Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if !files.is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("### Files\n");
        for file in files {
            let field = |name: &str| file.get(name).map(text_of).unwrap_or_default();
            let label = field("label");
            let label = if label.is_empty() {
                String::new()
            } else {
                format!(" ({label})")
            };
            let _ = write!(
                body,
                "\n- `{}`{label}: {}",
                field("filename"),
                field("changes_title")
            );
            let summary = field("changes_summary");
            for line in summary.lines().filter(|line| !line.trim().is_empty()) {
                let line = line.trim().trim_start_matches("- ");
                let _ = write!(body, "\n  - {line}");
            }
        }
    }
    body
}

/// A line diff of `old` and `new`: `  ` before a line in both, `- `
/// before a line of `old` only, `+ ` before a line of `new` only.
pub(crate) fn line_diff(old: &str, new: &str) -> Vec<String> {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    // common[i][j]: the longest common line sequence of old[i..] and new[j..].
    let mut common = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut lines = Vec::new();
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            lines.push(format!("  {}", old[i]));
            i += 1;
            j += 1;
        } else if common[i + 1][j] >= common[i][j + 1] {
            lines.push(format!("- {}", old[i]));
            i += 1;
        } else {
            lines.push(format!("+ {}", new[j]));
            j += 1;
        }
    }
    lines.extend(old[i..].iter().map(|line| format!("- {line}")));
    lines.extend(new[j..].iter().map(|line| format!("+ {line}")));
    lines
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
            backend: Backend::PrAgent,
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

    #[test]
    fn a_file_without_a_backend_is_from_pr_agent() {
        let text = r#"{"version": 1, "tool": "review", "pr": 7, "head_sha": "3a1f2c0",
            "model": "m", "started_at": "2026-09-21T14:13:20Z",
            "finished_at": "2026-09-21T14:13:20Z", "usage": {}, "data": {}}"#;
        let record: Record = serde_json::from_str(text).unwrap();
        assert_eq!(record.backend, Backend::PrAgent);
        let json = serde_json::to_value(Record {
            backend: Backend::OpenCodeReview,
            ..record
        })
        .unwrap();
        assert_eq!(json["backend"], "open_code_review");
    }

    #[test]
    fn backend_names_and_tools() {
        assert_eq!(Backend::default(), Backend::OpenCodeReview);
        assert_eq!(Backend::OpenCodeReview.name(), "ocr");
        assert_eq!(Backend::PrAgent.name(), "pr-agent");
        let ocr: Vec<_> = ToolKind::ALL
            .into_iter()
            .filter(|&kind| Backend::OpenCodeReview.supports(kind))
            .collect();
        assert_eq!(ocr, [ToolKind::Review]);
        assert!(
            ToolKind::ALL
                .into_iter()
                .all(|kind| Backend::PrAgent.supports(kind))
        );
    }

    #[test]
    fn ocr_review_gives_key_issues_the_most_severe_first() {
        let output = serde_json::json!({
            "status": "completed_with_warnings",
            "message": "Two findings.",
            "summary": {"files_reviewed": 9, "comments": 3, "total_tokens": 21344},
            "warnings": ["src/big.rs: timeout"],
            "comments": [
                {"path": "src/a.rs", "content": "Name is long.", "start_line": 9, "end_line": 9,
                 "category": "style", "severity": "low"},
                {"path": "src/foo.go", "content": "Concurrent map access without a lock.",
                 "start_line": 42, "end_line": 47, "category": "bug", "severity": "critical",
                 "existing_code": "m[k] = v", "suggestion_code": "mu.Lock(); m[k] = v",
                 "thinking": "Looking at line 42"},
                {"path": "src/b.rs", "content": "No line.", "start_line": 0, "end_line": 0},
                "not an object"
            ]
        });
        let data = ocr_review(&output);
        assert_eq!(
            data,
            serde_json::json!({"review": {
                "files_reviewed": 9,
                "summary": "Two findings.",
                "status": "completed_with_warnings",
                "warnings": 1,
                "key_issues_to_review": [
                    {"relevant_file": "src/foo.go", "issue_header": "Critical bug",
                     "issue_content": "Concurrent map access without a lock.",
                     "start_line": 42, "end_line": 47, "severity": "critical", "category": "bug",
                     "existing_code": "m[k] = v", "suggestion_code": "mu.Lock(); m[k] = v"},
                    {"relevant_file": "src/a.rs", "issue_header": "Low style",
                     "issue_content": "Name is long.", "start_line": 9, "end_line": 9,
                     "severity": "low", "category": "style"},
                    {"relevant_file": "src/b.rs", "issue_header": "Issue",
                     "issue_content": "No line."}
                ]
            }})
        );
        assert_eq!(
            review_markdown(&data),
            "- **Files reviewed:** 9\n\
             - **Status:** completed_with_warnings\n\
             - **Summary:** Two findings.\n\
             - **Warnings:** 1\n\
             \n\
             **Key issues**\n\
             \n\
             - **Critical bug** `src/foo.go:42-47`: Concurrent map access without a lock.\n\
             - **Low style** `src/a.rs:9`: Name is long.\n\
             - **Issue** `src/b.rs`: No line."
        );
        let skipped = serde_json::json!({"status": "skipped",
            "message": "No supported files changed.", "comments": []});
        assert_eq!(
            review_markdown(&ocr_review(&skipped)),
            "- **Status:** skipped\n- **Summary:** No supported files changed."
        );
    }

    #[test]
    fn short_tokens_cases() {
        let cases = [
            (0, "0"),
            (950, "950"),
            (1000, "1k"),
            (12_000, "12k"),
            (9800, "10k"),
        ];
        for (tokens, text) in cases {
            assert_eq!(short_tokens(tokens), text, "{tokens}");
        }
    }

    #[test]
    fn review_markdown_lists_the_fields_and_the_key_issues() {
        let data = serde_json::json!({"review": {
            "estimated_effort_to_review_[1-5]": 2,
            "relevant_tests": "No",
            "key_issues_to_review": [
                {"relevant_file": "src/lib.rs", "issue_header": "Possible Bug",
                 "issue_content": "It breaks.", "start_line": 3, "end_line": 5},
                {"relevant_file": "src/a.rs", "issue_header": "Style",
                 "issue_content": "Long.", "start_line": 9, "end_line": 9}
            ]
        }});
        assert_eq!(
            review_markdown(&data),
            "- **Estimated effort to review:** 2\n\
             - **Relevant tests:** No\n\
             \n\
             **Key issues**\n\
             \n\
             - **Possible Bug** `src/lib.rs:3-5`: It breaks.\n\
             - **Style** `src/a.rs:9`: Long."
        );
        assert_eq!(review_markdown(&serde_json::json!({})), "");
    }

    #[test]
    fn describe_gives_the_title_and_a_body_with_a_walkthrough() {
        let data = serde_json::json!({
            "title": "Add a greeting",
            "description": "- Add a greeting function\n- Test it",
            "pr_files": [{"filename": "src/lib.rs", "changes_title": "Add greet",
                          "changes_summary": "- Add `greet`\n- Trim", "label": "enhancement"}]
        });
        assert_eq!(describe_title(&data).as_deref(), Some("Add a greeting"));
        assert_eq!(
            describe_body(&data),
            "- Add a greeting function\n- Test it\n\n### Files\n\
             \n- `src/lib.rs` (enhancement): Add greet\n  - Add `greet`\n  - Trim"
        );
        assert_eq!(describe_title(&serde_json::json!({"title": " "})), None);
        assert_eq!(describe_body(&serde_json::json!({})), "");
    }

    #[test]
    fn line_diff_marks_the_kept_the_old_and_the_new_lines() {
        assert_eq!(
            line_diff("a\nb\nc", "a\nx\nc\nd"),
            ["  a", "- b", "+ x", "  c", "+ d"]
        );
        assert_eq!(line_diff("", "new"), ["+ new"]);
        assert_eq!(line_diff("old", ""), ["- old"]);
        assert_eq!(line_diff("same", "same"), ["  same"]);
    }

    #[test]
    fn suggestions_parse_and_skip_bad_items() {
        let data = serde_json::json!({"code_suggestions": [
            {"relevant_file": "src/lib.rs", "relevant_lines_start": 4, "relevant_lines_end": 4,
             "one_sentence_summary": "Trim", "label": "possible issue", "score": 8},
            "not an object"
        ]});
        let found = suggestions(&data);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].relevant_file, "src/lib.rs");
        assert_eq!(found[0].score, Some(serde_json::json!(8)));
    }
}
