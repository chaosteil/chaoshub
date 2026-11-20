//! The actions that copy the url, the urls of the stack, or the change id
//! or branch name.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Group, Prepare, STACK_NOT_LOADED, Services};
use crate::bus::{ActionInput, Cmd, KeyChord, KeyCode, KeyModifiers, Selection, Target};
use crate::config::schema::{CopyStackUrlsConfig, StackUrlsOrder};
use crate::domain::local::{LocalMatch, Vcs};
use crate::domain::pr::PrState;

/// The action that copies the url of a pull request.
pub(crate) struct CopyUrl;

/// The action that copies the urls of the open pull requests of a stack,
/// one url on each line. The order is the order of the Stack pane:
/// position 1, the bottom of the stack, first.
/// `[actions.copy_stack_urls] order = "top_first"` puts the top of the
/// stack first, as the list does.
pub(crate) struct CopyStackUrls;

/// The action that copies the jj change id of a pull request. In a git
/// repo, it copies the local branch name.
pub(crate) struct CopyChangeId;

/// A key without modifiers.
fn key(c: char) -> KeyChord {
    KeyChord {
        code: KeyCode::Char(c),
        mods: KeyModifiers::NONE,
    }
}

/// The urls of the stack of a target.
#[derive(Debug, PartialEq)]
enum StackUrls {
    /// The pull request has no stack, or a stack of one pull request.
    NoStack,
    /// An entry of the stack that did not merge is not loaded.
    NotLoaded,
    /// The urls of the open entries, in the order of the configuration.
    Loaded(Vec<String>),
}

/// The urls of the stack of the pull request of `target`, in `order`. A
/// merged entry does not have to be loaded, because the stack names the
/// merged positions. The list shows only open pull requests, so the urls
/// skip a merged or closed entry. The pull request of the target is
/// always in the urls.
fn stack_urls(target: &Target, order: StackUrlsOrder) -> StackUrls {
    let Target::Pr { pr, stack, .. } = target;
    let Some(stack_ref) = pr.head.value.stack.as_ref().filter(|s| s.size >= 2) else {
        return StackUrls::NoStack;
    };
    let mut urls = Vec::new();
    for position in 1..=stack_ref.size.max(stack_ref.position) {
        if stack_ref.merged.contains(&position) && position != stack_ref.position {
            continue;
        }
        let entry = if position == stack_ref.position {
            Some(pr)
        } else {
            stack.iter().find(|entry| {
                entry
                    .head
                    .value
                    .stack
                    .as_ref()
                    .is_some_and(|s| s.position == position)
            })
        };
        let Some(entry) = entry else {
            return StackUrls::NotLoaded;
        };
        if entry.key == pr.key || entry.head.value.state == PrState::Open {
            urls.push(entry.head.value.url.clone());
        }
    }
    if order == StackUrlsOrder::TopFirst {
        urls.reverse();
    }
    StackUrls::Loaded(urls)
}

/// The order of the stack urls, from the `[actions.copy_stack_urls]`
/// table. The default without a table. `Config::parse` stops at a bad
/// value, so a table that does not parse also gives the default.
fn stack_urls_order(sel: &Selection) -> StackUrlsOrder {
    sel.action_config
        .clone()
        .and_then(|table| toml::Value::Table(table).try_into().ok())
        .map(|config: CopyStackUrlsConfig| config.order)
        .unwrap_or_default()
}

/// The change id of the local change that matches the pull request of the
/// target. In a git repo, the branch name. `None` for a pull request
/// without a local change.
fn change_id(target: &Target) -> Option<&str> {
    let Target::Pr { local, .. } = target;
    match local {
        LocalMatch::Synced { change_id, .. } | LocalMatch::Diverged { change_id, .. } => {
            Some(change_id)
        }
        LocalMatch::None => None,
    }
}

impl Action for CopyUrl {
    fn id(&self) -> &'static str {
        "copy_url"
    }

    fn label(&self) -> &'static str {
        "copy the url"
    }

    fn hint(&self) -> String {
        "copy URL".to_owned()
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(key('y'))
    }

    fn group(&self) -> Group {
        Group::Safe
    }

    /// A pull request.
    fn applies(&self, _sel: &Selection) -> Applicability {
        Applicability::Enabled
    }

    fn prepare(&self, sel: &Selection) -> Prepare {
        let Target::Pr { pr, .. } = &sel.target;
        Prepare::Command(Cmd::Copy(pr.head.value.url.clone()))
    }

    /// The url goes to the clipboard through `Cmd::Copy`; the runtime
    /// never calls this.
    fn execute(
        &self,
        _sel: Selection,
        _input: ActionInput,
        _svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async { Err("the copy runs through a command".to_owned()) })
    }
}

impl Action for CopyStackUrls {
    fn id(&self) -> &'static str {
        "copy_stack_urls"
    }

    fn label(&self) -> &'static str {
        "copy the stack urls"
    }

    fn hint(&self) -> String {
        "copy stack URLs".to_owned()
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(key('Y'))
    }

    fn group(&self) -> Group {
        Group::Safe
    }

    /// A pull request in a stack of two or more pull requests. Disabled
    /// when an entry of the stack is not loaded. The app then refreshes
    /// the stack, as for `merge_stack`.
    fn applies(&self, sel: &Selection) -> Applicability {
        match stack_urls(&sel.target, StackUrlsOrder::default()) {
            StackUrls::NoStack => Applicability::Hidden,
            StackUrls::NotLoaded => Applicability::Disabled(STACK_NOT_LOADED),
            StackUrls::Loaded(_) => Applicability::Enabled,
        }
    }

    /// The urls in the order of `[actions.copy_stack_urls] order`.
    fn prepare(&self, sel: &Selection) -> Prepare {
        let urls = match stack_urls(&sel.target, stack_urls_order(sel)) {
            StackUrls::Loaded(urls) => urls,
            StackUrls::NoStack | StackUrls::NotLoaded => Vec::new(),
        };
        Prepare::Command(Cmd::Copy(urls.join("\n")))
    }

    /// The urls go to the clipboard through `Cmd::Copy`; the runtime
    /// never calls this.
    fn execute(
        &self,
        _sel: Selection,
        _input: ActionInput,
        _svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async { Err("the copy runs through a command".to_owned()) })
    }
}

impl Action for CopyChangeId {
    fn id(&self) -> &'static str {
        "copy_change_id"
    }

    fn label(&self) -> &'static str {
        "copy the change id"
    }

    /// "copy the branch name" for a git branch.
    fn target_label(&self, target: &Target) -> &'static str {
        let Target::Pr { local, .. } = target;
        match local {
            LocalMatch::Synced { vcs: Vcs::Git, .. }
            | LocalMatch::Diverged { vcs: Vcs::Git, .. } => "copy the branch name",
            _ => self.label(),
        }
    }

    /// "copy branch" for a git branch.
    fn target_hint(&self, target: &Target) -> String {
        let Target::Pr { local, .. } = target;
        match local {
            LocalMatch::Synced { vcs: Vcs::Git, .. }
            | LocalMatch::Diverged { vcs: Vcs::Git, .. } => "copy branch".to_owned(),
            _ => self.hint(),
        }
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(key('i'))
    }

    fn group(&self) -> Group {
        Group::Safe
    }

    /// A pull request with a local change.
    fn applies(&self, sel: &Selection) -> Applicability {
        match change_id(&sel.target) {
            Some(_) => Applicability::Enabled,
            None => Applicability::Hidden,
        }
    }

    fn prepare(&self, sel: &Selection) -> Prepare {
        let id = change_id(&sel.target).unwrap_or_default();
        Prepare::Command(Cmd::Copy(id.to_owned()))
    }

    /// The change id or the branch name goes to the clipboard through
    /// `Cmd::Copy`; the runtime never calls this.
    fn execute(
        &self,
        _sel: Selection,
        _input: ActionInput,
        _svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async { Err("the copy runs through a command".to_owned()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pr::PullRequest;
    use crate::domain::repo::Repo;

    fn selection(local: LocalMatch) -> Selection {
        let pr: PullRequest = serde_json::from_str(include_str!("testdata/pr.json")).unwrap();
        Selection {
            target: Target::Pr {
                repo: Repo {
                    key: pr.key.repo.clone(),
                    path: Some("/code/chaoshub".into()),
                    settings: None,
                },
                pr,
                stack: Vec::new(),
                local,
            },
            me: "chaosteil".into(),
            action_config: None,
        }
    }

    #[test]
    fn y_copies_the_url_of_a_pull_request() {
        let sel = selection(LocalMatch::None);
        assert_eq!(CopyUrl.applies(&sel), Applicability::Enabled);
        assert_eq!(
            CopyUrl.prepare(&sel),
            Prepare::Command(Cmd::Copy(
                "https://github.com/chaosteil/chaoshub/pull/7".into()
            ))
        );
    }

    #[test]
    fn i_copies_the_change_id_of_a_match() {
        let copy = |sel: &Selection| (CopyChangeId.applies(sel), CopyChangeId.prepare(sel));
        let enabled = |id: &str| {
            (
                Applicability::Enabled,
                Prepare::Command(Cmd::Copy(id.to_owned())),
            )
        };
        let synced = selection(LocalMatch::Synced {
            workspace: "default".into(),
            change_id: "kpqvxwyz".into(),
            commit_id: "oid7".into(),
            vcs: Vcs::Jj,
        });
        assert_eq!(copy(&synced), enabled("kpqvxwyz"));
        let diverged = selection(LocalMatch::Diverged {
            workspace: "agent".into(),
            change_id: "mnoplqrs".into(),
            commit_id: "c0ffee".into(),
            vcs: Vcs::Jj,
        });
        assert_eq!(copy(&diverged), enabled("mnoplqrs"));
        assert_eq!(
            CopyChangeId.applies(&selection(LocalMatch::None)),
            Applicability::Hidden
        );
    }

    #[test]
    fn i_copies_the_branch_name_in_a_git_repo() {
        let git = selection(LocalMatch::Diverged {
            workspace: "default".into(),
            change_id: "chaosteil/export".into(),
            commit_id: "c0ffee".into(),
            vcs: Vcs::Git,
        });
        assert_eq!(CopyChangeId.applies(&git), Applicability::Enabled);
        assert_eq!(
            CopyChangeId.prepare(&git),
            Prepare::Command(Cmd::Copy("chaosteil/export".into()))
        );
        assert_eq!(
            CopyChangeId.target_label(&git.target),
            "copy the branch name"
        );
        let jj = selection(LocalMatch::Synced {
            workspace: "default".into(),
            change_id: "kpqvxwyz".into(),
            commit_id: "oid7".into(),
            vcs: Vcs::Jj,
        });
        assert_eq!(CopyChangeId.target_hint(&git.target), "copy branch");
        assert_eq!(CopyChangeId.target_label(&jj.target), "copy the change id");
        assert_eq!(CopyChangeId.target_hint(&jj.target), "copy change id");
        assert_eq!(
            CopyChangeId.target_label(&selection(LocalMatch::None).target),
            "copy the change id"
        );
    }

    /// The pull request `number` at `position` of the stack `s` of three
    /// pull requests, with a url.
    fn entry(number: u64, position: u32) -> PullRequest {
        let mut pr = crate::domain::matching::tests::pr(number, "s", position);
        pr.head.value.url = format!("https://github.com/o/r/pull/{number}");
        pr
    }

    /// The selection of `pr` with the loaded stack entries `stack`.
    fn stack_selection(pr: PullRequest, stack: Vec<PullRequest>) -> Selection {
        let mut sel = selection(LocalMatch::None);
        let Target::Pr {
            pr: own,
            stack: entries,
            ..
        } = &mut sel.target;
        *own = pr;
        *entries = stack;
        sel
    }

    #[test]
    fn shift_y_copies_the_stack_urls_bottom_first() {
        let stack = vec![entry(11, 1), entry(12, 2), entry(13, 3)];
        for selected in &stack {
            let sel = stack_selection(selected.clone(), stack.clone());
            assert_eq!(CopyStackUrls.applies(&sel), Applicability::Enabled);
            assert_eq!(
                CopyStackUrls.prepare(&sel),
                Prepare::Command(Cmd::Copy(
                    "https://github.com/o/r/pull/11\n\
                     https://github.com/o/r/pull/12\n\
                     https://github.com/o/r/pull/13"
                        .into()
                )),
                "#{}",
                selected.key.number
            );
        }
    }

    #[test]
    fn shift_y_copies_the_stack_urls_in_the_order_of_the_configuration() {
        let stack = vec![entry(11, 1), entry(12, 2), entry(13, 3)];
        let mut sel = stack_selection(entry(12, 2), stack);
        let copied = |sel: &Selection| {
            let Prepare::Command(Cmd::Copy(text)) = CopyStackUrls.prepare(sel) else {
                unreachable!()
            };
            text
        };
        let bottom_first = "https://github.com/o/r/pull/11\n\
                            https://github.com/o/r/pull/12\n\
                            https://github.com/o/r/pull/13";
        let top_first = "https://github.com/o/r/pull/13\n\
                         https://github.com/o/r/pull/12\n\
                         https://github.com/o/r/pull/11";
        for (config, expected) in [
            ("", bottom_first),
            ("order = \"bottom_first\"", bottom_first),
            ("order = \"top_first\"", top_first),
        ] {
            sel.action_config = Some(toml::from_str(config).unwrap());
            assert_eq!(copied(&sel), expected, "{config}");
        }
    }

    #[test]
    fn shift_y_skips_the_merged_entries_of_the_stack() {
        let mut merged = entry(11, 1);
        merged.head.value.state = PrState::Merged;
        let mut top = entry(13, 3);
        let mut middle = entry(12, 2);
        for pr in [&mut top, &mut middle] {
            pr.head.value.stack.as_mut().unwrap().merged = vec![1];
        }
        let copied = Prepare::Command(Cmd::Copy(
            "https://github.com/o/r/pull/12\nhttps://github.com/o/r/pull/13".into(),
        ));
        let loaded = stack_selection(top.clone(), vec![merged, middle.clone(), top.clone()]);
        assert_eq!(CopyStackUrls.prepare(&loaded), copied);
        let not_loaded = stack_selection(top.clone(), vec![middle, top]);
        assert_eq!(CopyStackUrls.applies(&not_loaded), Applicability::Enabled);
        assert_eq!(CopyStackUrls.prepare(&not_loaded), copied);
    }

    #[test]
    fn shift_y_waits_for_an_open_entry_that_is_not_loaded() {
        let stack = vec![entry(11, 1), entry(13, 3)];
        let sel = stack_selection(entry(13, 3), stack);
        assert_eq!(
            CopyStackUrls.applies(&sel),
            Applicability::Disabled(STACK_NOT_LOADED)
        );
    }

    #[test]
    fn shift_y_is_hidden_without_a_stack() {
        assert_eq!(
            CopyStackUrls.applies(&selection(LocalMatch::None)),
            Applicability::Hidden
        );
        let mut lone = entry(11, 1);
        lone.head.value.stack.as_mut().unwrap().size = 1;
        let sel = stack_selection(lone.clone(), vec![lone]);
        assert_eq!(CopyStackUrls.applies(&sel), Applicability::Hidden);
    }
}
