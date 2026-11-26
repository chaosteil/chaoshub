//! The state that a resumed chaoshub reads back, for OSC 7577 (program
//! resume). No IO.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::keys::PrKey;

/// The env that the terminal sets when it supports OSC 7577. chaoshub
/// never gives it to a child.
pub(crate) const TOKEN_ENV: &str = "OSC7577_TOKEN";

/// The env that gives the instance id of the list to its PR views.
pub(crate) const LIST_ID_ENV: &str = "CHAOSHUB_LIST_ID";

/// The version of the state file format. A file of another version is
/// ignored.
pub(crate) const STATE_VERSION: u32 = 1;

/// The state of one instance: a list or a PR view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ResumeState {
    List(ListResume),
    PrView(PrViewResume),
}

/// The state of a list: the active tab, the state of each tab and the
/// open PR view panes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ListResume {
    /// The id of the active tab.
    pub tab: String,
    pub tabs: Vec<TabResume>,
    pub panes: Vec<PaneResume>,
}

/// The state of one tab: the selected PR and the filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TabResume {
    /// The tab id, for example `mine`.
    pub id: String,
    pub selected: Option<PrKey>,
    pub filter: String,
}

/// A PR view pane of a list: the PR and the Rex block of the pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PaneResume {
    pub pr: PrKey,
    pub block_id: String,
    pub label: String,
}

/// The state of a PR view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PrViewResume {
    pub pr: PrKey,
    /// The id of the active pane, for example `checks`.
    pub pane: String,
    /// The instance id of the list that started the PR view.
    pub list: Option<String>,
    /// The socket of that list.
    pub list_socket: Option<PathBuf>,
    /// True when the PR view renames its Rex block.
    pub rex_label: bool,
    /// The Rex block of the PR view.
    pub block: Option<String>,
}

/// The UI part of the state, from app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum UiResume {
    List { tab: String, tabs: Vec<TabResume> },
    PrView { pane: String },
}

/// The panes that a resumed list adopts: its own panes, with the block
/// of a PR view file of the same PR when it has one, then the PR views of
/// the list that it did not know. One per PR.
///
/// `views` are the PR views of the list, the newest first. A PR view
/// writes its block at each start, so its block is newer than the block
/// in the file of the list.
pub(crate) fn adoption(list: &ListResume, views: &[PrViewResume]) -> Vec<PaneResume> {
    let mut panes: Vec<PaneResume> = Vec::new();
    for pane in &list.panes {
        if panes.iter().any(|known| known.pr == pane.pr) {
            continue;
        }
        let block = views
            .iter()
            .filter(|view| view.pr == pane.pr)
            .find_map(|view| view.block.clone());
        panes.push(PaneResume {
            pr: pane.pr.clone(),
            block_id: block.unwrap_or_else(|| pane.block_id.clone()),
            label: pane.label.clone(),
        });
    }
    for view in views {
        let Some(block) = &view.block else {
            continue;
        };
        if panes.iter().any(|known| known.pr == view.pr) {
            continue;
        }
        panes.push(PaneResume {
            pr: view.pr.clone(),
            block_id: block.clone(),
            label: String::new(),
        });
    }
    panes
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::domain::assert_round_trip;
    use crate::domain::keys::RepoKey;

    fn pr(number: u64) -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: "chaosteil".into(),
                name: "chaoshub".into(),
            },
            number,
        }
    }

    fn pane(number: u64, block: &str) -> PaneResume {
        PaneResume {
            pr: pr(number),
            block_id: block.into(),
            label: format!("#{number}"),
        }
    }

    fn view(number: u64, block: Option<&str>) -> PrViewResume {
        PrViewResume {
            pr: pr(number),
            pane: "description".into(),
            list: Some("0123456789abcdef".into()),
            list_socket: Some(PathBuf::from("/run/chaoshub/list-0123456789abcdef.sock")),
            rex_label: true,
            block: block.map(str::to_owned),
        }
    }

    fn list(panes: Vec<PaneResume>) -> ListResume {
        ListResume {
            tab: "review".into(),
            tabs: vec![TabResume {
                id: "review".into(),
                selected: Some(pr(7)),
                filter: "fix".into(),
            }],
            panes,
        }
    }

    #[test]
    fn resume_state_round_trips() {
        assert_round_trip(&ResumeState::List(list(vec![pane(7, "block:02")])));
        assert_round_trip(&ResumeState::PrView(view(7, Some("block:02"))));
        assert_round_trip(&ResumeState::PrView(PrViewResume {
            list: None,
            list_socket: None,
            block: None,
            ..view(7, None)
        }));
        let json = serde_json::to_value(ResumeState::PrView(view(7, None))).unwrap();
        assert_eq!(json["kind"], "pr_view");
    }

    #[test]
    fn adoption_takes_the_newest_block_of_each_pr_view() {
        let panes = adoption(
            &list(vec![pane(7, "block:02"), pane(8, "block:03")]),
            &[view(7, Some("block:12")), view(7, Some("block:22"))],
        );
        assert_eq!(panes, [pane(7, "block:12"), pane(8, "block:03")]);
    }

    #[test]
    fn adoption_adds_the_pr_views_that_the_list_did_not_know() {
        let panes = adoption(
            &list(vec![pane(7, "block:02")]),
            &[view(9, Some("block:09")), view(10, None)],
        );
        assert_eq!(
            panes,
            [
                pane(7, "block:02"),
                PaneResume {
                    label: String::new(),
                    ..pane(9, "block:09")
                },
            ]
        );
    }

    #[test]
    fn adoption_has_one_pane_per_pr() {
        let panes = adoption(
            &list(vec![pane(7, "block:02"), pane(7, "block:05")]),
            &[view(9, Some("block:09")), view(9, Some("block:19"))],
        );
        let prs: Vec<u64> = panes.iter().map(|pane| pane.pr.number).collect();
        assert_eq!(prs, [7, 9]);
        assert_eq!(panes[0].block_id, "block:02");
        assert_eq!(panes[1].block_id, "block:09");
    }
}
