//! The Agent pane of the pull request popup: one section per pr-agent
//! tool, with its state, the age of the last run, the model and the head
//! commit.
//!
//! The keys `r`, `i`, `d` and `a` start review, improve, describe and
//! ask. `a` asks for the question first. The pane keys win over the
//! action keys, so `d` (draft) does not work while the pane is selected.

use crate::app::age::age;
use crate::app::frame::spinner;
use crate::app::models::{Cell, CellStyle, PaneModel};
use crate::app::panes::{Pane, PaneState};
use crate::app::state::AppData;
use crate::app::tabs::cells::cell;
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{AgentJob, Cmd, KeyChord, KeyCode, Size, Target};
use crate::domain::agent::{
    AgentResults, Record, Tool, ToolKind, describe_body, describe_title, review_markdown,
    suggestions,
};
use crate::domain::pr::PullRequest;

/// The Agent pane.
pub(crate) struct AgentPane;

impl Pane for AgentPane {
    fn id(&self) -> &'static str {
        "agent"
    }

    fn title(&self) -> &'static str {
        "Agent"
    }

    fn key(&self) -> char {
        '6'
    }

    /// A pull request of a repo with a local path: pr-agent runs in it.
    fn applies(&self, target: &Target) -> bool {
        matches!(target, Target::Pr { repo, .. } if repo.path.is_some())
    }

    fn init_state(&self) -> PaneState {
        PaneState::default()
    }

    fn model(&self, cx: &KeyCx<'_>, _ps: &PaneState, _size: Size) -> PaneModel {
        let Some(Target::Pr { pr, .. }) = cx.target else {
            return PaneModel::Lines {
                lines: Vec::new(),
                wrap: true,
            };
        };
        let head = head(cx.data, pr);
        let empty = AgentResults::default();
        let results = cx.data.agent.get(&pr.key).unwrap_or(&empty);
        let mut lines = Vec::new();
        for kind in ToolKind::ALL {
            if !lines.is_empty() {
                lines.push(Vec::new());
            }
            let running = cx.data.agent_jobs.contains(&(pr.key.clone(), kind));
            section(&mut lines, cx, kind, results, running, &head);
        }
        PaneModel::Lines { lines, wrap: true }
    }

    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, _ps: &mut PaneState) -> Option<Handled> {
        if !key.mods.is_empty() {
            return None;
        }
        let tool = match key.code {
            KeyCode::Char('r') => Tool::Review,
            KeyCode::Char('i') => Tool::Improve,
            KeyCode::Char('d') => Tool::Describe,
            KeyCode::Char('a') => Tool::Ask(String::new()),
            _ => return None,
        };
        let Some(Target::Pr { pr, repo, .. }) = cx.target else {
            return None;
        };
        let job = AgentJob {
            key: pr.key.clone(),
            head_sha: head(cx.data, pr),
            repo_path: repo.path.clone()?,
            tool,
        };
        Some(Handled {
            cmds: vec![Cmd::RunAgent(job)],
            ..Handled::default()
        })
    }
}

/// The head commit of the pull request: from the loaded conversation
/// (`PrDetail.head_oid`), else from the pull request.
pub(crate) fn head(data: &AppData, pr: &PullRequest) -> String {
    data.details.get(&pr.key).map_or_else(
        || pr.head.value.head_oid.clone(),
        |detail| detail.value.head_oid.clone(),
    )
}

/// The first 7 characters of a commit id.
fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// The title and the key of a tool.
fn title(kind: ToolKind) -> (&'static str, char) {
    match kind {
        ToolKind::Review => ("Review", 'r'),
        ToolKind::Improve => ("Improve", 'i'),
        ToolKind::Describe => ("Describe", 'd'),
        ToolKind::Ask => ("Ask", 'a'),
    }
}

/// Adds the lines of the section of `kind`.
fn section(
    lines: &mut Vec<Vec<Cell>>,
    cx: &KeyCx<'_>,
    kind: ToolKind,
    results: &AgentResults,
    running: bool,
    head: &str,
) {
    let (name, key) = title(kind);
    let record = results.records.get(&kind);
    let error = results.errors.get(&kind);
    let mut header = vec![
        cell(name, CellStyle::Primary),
        cell(format!(" [{key}] · "), CellStyle::Dim),
    ];
    header.push(if running {
        cell(format!("{} running", spinner(cx.data.now)), CellStyle::Warn)
    } else if error.is_some() {
        cell("failed", CellStyle::Danger)
    } else if let Some(record) = record {
        cell(
            format!("done {} ago", age(record.finished_at, cx.data.now)),
            CellStyle::Success,
        )
    } else {
        cell("never", CellStyle::Dim)
    });
    if let Some(record) = record {
        header.push(cell(
            format!(" · {} · head {}", record.model, short(&record.head_sha)),
            CellStyle::Dim,
        ));
    }
    lines.push(header);
    if let Some(error) = error {
        lines.push(vec![cell(error.clone(), CellStyle::Danger)]);
    }
    let Some(record) = record else {
        return;
    };
    if record.head_sha != head {
        lines.push(vec![cell(
            format!(
                "outdated: for {}, the head is now {}",
                short(&record.head_sha),
                short(head)
            ),
            CellStyle::Warn,
        )]);
    }
    body(lines, cx, kind, record);
}

/// Adds the result of a record.
fn body(lines: &mut Vec<Vec<Cell>>, cx: &KeyCx<'_>, kind: ToolKind, record: &Record) {
    let markdown = |text: String| vec![cell(text, CellStyle::Markdown)];
    let none = |text: &str| vec![cell(text, CellStyle::Dim)];
    match kind {
        ToolKind::Review => {
            let text = review_markdown(&record.data);
            lines.push(if text.is_empty() {
                none("(no review)")
            } else {
                markdown(text)
            });
        }
        ToolKind::Improve => {
            let found = suggestions(&record.data);
            if found.is_empty() {
                lines.push(none("(no suggestions)"));
            }
            for suggestion in found {
                lines.push(vec![
                    cell(suggestion.label, CellStyle::Info),
                    cell(
                        format!(
                            " · {}:{} · ",
                            suggestion.relevant_file, suggestion.relevant_lines_start
                        ),
                        CellStyle::Dim,
                    ),
                    cell(suggestion.one_sentence_summary, CellStyle::Normal),
                ]);
            }
        }
        ToolKind::Describe => {
            let title = describe_title(&record.data).unwrap_or_default();
            let body = describe_body(&record.data);
            lines.push(markdown(format!("**{title}**\n\n{body}")));
        }
        ToolKind::Ask => {
            for entry in record.ask_entries() {
                lines.push(vec![cell(
                    format!(
                        "{} ago · head {}",
                        age(entry.at, cx.data.now),
                        short(&entry.head_sha)
                    ),
                    CellStyle::Dim,
                )]);
                lines.push(markdown(format!(
                    "**Q:** {}\n\n{}",
                    entry.question, entry.answer
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::target::pr_target;
    use crate::app::testdata;
    use crate::bus::KeyModifiers;
    use crate::domain::agent::VERSION;
    use crate::domain::repo::Repo;
    use crate::domain::test_time;

    /// A record of `kind` for the head `head`, finished at the test time.
    fn record(kind: ToolKind, head: &str, data: serde_json::Value) -> Record {
        Record {
            version: VERSION,
            tool: kind.name().into(),
            pr: 7,
            head_sha: head.into(),
            model: "anthropic/claude-sonnet-5-5".into(),
            started_at: test_time(),
            finished_at: test_time(),
            usage: serde_json::json!({"total_tokens": 12000}),
            data,
        }
    }

    /// A state with #7 of `chaosteil/chaoshub` (head `oid7`) in a repo at
    /// `/code/chaoshub`, five minutes after the test time.
    fn state() -> AppState {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        let pr = testdata::pr("chaoshub", 7, "Seven");
        state.data.repos.insert(
            pr.key.repo.clone(),
            Repo {
                key: pr.key.repo.clone(),
                path: Some("/code/chaoshub".into()),
                settings: None,
            },
        );
        state.data.prs.insert(pr.key.clone(), pr);
        state.data.now = test_time() + TimeDelta::minutes(5);
        state
    }

    /// The text of each line of the pane.
    fn texts(state: &AppState) -> Vec<String> {
        let target = pr_target(&state.data, &testdata::pr_key("chaoshub", 7)).unwrap();
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let PaneModel::Lines { lines, .. } =
            AgentPane.model(&cx, &PaneState::default(), state.data.size)
        else {
            panic!("not lines");
        };
        lines
            .iter()
            .map(|cells| cells.iter().map(|c| c.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn a_pull_request_without_results_shows_never() {
        assert_eq!(
            texts(&state()),
            [
                "Review [r] · never",
                "",
                "Improve [i] · never",
                "",
                "Describe [d] · never",
                "",
                "Ask [a] · never",
            ]
        );
    }

    #[test]
    fn the_sections_show_the_results_the_states_and_outdated_heads() {
        let mut state = state();
        let key = testdata::pr_key("chaoshub", 7);
        let review = serde_json::json!({"review": {"key_issues_to_review": [
            {"relevant_file": "src/lib.rs", "issue_header": "Possible Bug",
             "issue_content": "It breaks.", "start_line": 3, "end_line": 5}
        ]}});
        let improve = serde_json::json!({"code_suggestions": [
            {"relevant_file": "src/lib.rs", "relevant_lines_start": 4, "relevant_lines_end": 4,
             "one_sentence_summary": "Trim the name", "label": "possible issue"}
        ]});
        let ask = serde_json::json!([{"question": "Why?", "answer": "Because.",
            "head_sha": "oid7", "at": "2026-09-21T14:13:20Z"}]);
        let results = AgentResults {
            records: [
                (
                    ToolKind::Review,
                    record(ToolKind::Review, "3a1f2c0aa", review),
                ),
                (
                    ToolKind::Improve,
                    record(ToolKind::Improve, "oid7", improve),
                ),
                (ToolKind::Ask, record(ToolKind::Ask, "oid7", ask)),
            ]
            .into(),
            errors: [(ToolKind::Describe, "pr-agent failed: no key".to_owned())].into(),
        };
        state.data.agent.insert(key.clone(), results);
        state.data.agent_jobs.insert((key, ToolKind::Improve));
        let lines = texts(&state);
        assert_eq!(
            lines[..3],
            [
                "Review [r] · done 5m ago · anthropic/claude-sonnet-5-5 · head 3a1f2c0",
                "outdated: for 3a1f2c0, the head is now oid7",
                "**Key issues**\n\n- **Possible Bug** `src/lib.rs:3-5`: It breaks.",
            ]
        );
        assert!(lines[4].starts_with("Improve [i] · "), "{}", lines[4]);
        assert!(lines[4].contains(" running · "), "{}", lines[4]);
        assert_eq!(lines[5], "possible issue · src/lib.rs:4 · Trim the name");
        assert_eq!(lines[7], "Describe [d] · failed");
        assert_eq!(lines[8], "pr-agent failed: no key");
        assert_eq!(lines[11], "5m ago · head oid7");
        assert_eq!(lines[12], "**Q:** Why?\n\nBecause.");
    }

    #[test]
    fn the_tool_keys_start_runs_at_the_head() {
        let state = state();
        let target = pr_target(&state.data, &testdata::pr_key("chaoshub", 7)).unwrap();
        assert!(AgentPane.applies(&target));
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let press = |c: char| {
            let chord = KeyChord {
                code: KeyCode::Char(c),
                mods: KeyModifiers::NONE,
            };
            AgentPane
                .handle_key(chord, &cx, &mut PaneState::default())
                .map(|handled| handled.cmds)
        };
        let job = |tool| {
            vec![Cmd::RunAgent(AgentJob {
                key: testdata::pr_key("chaoshub", 7),
                head_sha: "oid7".into(),
                repo_path: "/code/chaoshub".into(),
                tool,
            })]
        };
        assert_eq!(press('r'), Some(job(Tool::Review)));
        assert_eq!(press('i'), Some(job(Tool::Improve)));
        assert_eq!(press('d'), Some(job(Tool::Describe)));
        assert_eq!(press('a'), Some(job(Tool::Ask(String::new()))));
        assert_eq!(press('x'), None);
    }
}
