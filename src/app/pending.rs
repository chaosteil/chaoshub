//! The pending map: the parts of pull requests that an action changes on
//! GitHub now. A part pulses while it is in the map.
//!
//! An action declares its parts and the change that each part waits for
//! (`Action::pending`). A part goes into the map when `update` sends
//! `Cmd::RunAction`. The part stays while the action runs. After a
//! success, the part stays until the GitHub data shows the change: data
//! from after the result that does not show the change yet keeps the
//! part. A failure removes the part at once. This is the rollback: the
//! views then show the GitHub data again, and the status line shows the
//! error.
//!
//! Two limits make sure that a part never stays: `SETTLE_LIMIT` after a
//! success, and `RUN_LIMIT` after the start of an action whose result
//! never comes.
//!
//! After a success, the map asks for new reads of the pull request
//! (`REREAD_WAITS`), until its parts settled and GitHub knows its merge
//! state. GitHub can need some seconds for both.
//!
//! The parts also replace the GitHub data in some views until they end:
//! a pending approval hides the pull request from the Review tab, and a
//! pending reaction shows in the conversation.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use chrono::{DateTime, TimeDelta, Utc};

use crate::bus::{Expect, Pending, PendingPart, ReviewKind};
use crate::domain::detail::{PrDetail, Reaction, ReactionKind, ReviewState, has_mine};
use crate::domain::keys::PrKey;
use crate::domain::observed::Observed;
use crate::domain::pr::{MergeStateStatus, Mergeable, PrState, PullRequest};

/// How long a part stays after the success of its action when the GitHub
/// data does not show the change.
pub(crate) const SETTLE_LIMIT: TimeDelta = TimeDelta::seconds(60);

/// How long a part stays while its action runs. A stack merge can poll
/// for many minutes, so the limit is long. It is only for a result that
/// never comes.
pub(crate) const RUN_LIMIT: TimeDelta = TimeDelta::minutes(30);

/// The waits between the new reads of a pull request after a success:
/// the first read comes 2 s after the success. The reads stop early when
/// the parts of the pull request settled and its merge state is known.
pub(crate) const REREAD_WAITS: [TimeDelta; 5] = [
    TimeDelta::seconds(2),
    TimeDelta::seconds(5),
    TimeDelta::seconds(15),
    TimeDelta::seconds(15),
    TimeDelta::seconds(15),
];

/// The GitHub data that the predicates of the parts read.
#[derive(Debug, Clone, Copy)]
pub(crate) struct View<'a> {
    pub prs: &'a BTreeMap<PrKey, PullRequest>,
    pub details: &'a BTreeMap<PrKey, Observed<PrDetail>>,
    /// The pull requests whose merge state GitHub computes now
    /// (`AppData::computing`).
    pub computing: &'a BTreeMap<PrKey, DateTime<Utc>>,
}

/// The parts that actions change on GitHub now, the runs that wait for
/// their results, and the new reads after a success.
#[derive(Debug, Default)]
pub(crate) struct PendingMap {
    entries: BTreeMap<(PrKey, PendingPart), Entry>,
    /// The runs that wait for their result, by action id and by the pull
    /// request of the target, oldest first.
    runs: BTreeMap<(&'static str, PrKey), VecDeque<Run>>,
    /// The parts of the next result of an action on a pull request, from
    /// `ActionEvent::Parts`. They pick the run of the result.
    next: BTreeMap<(&'static str, PrKey), Vec<Pending>>,
    rereads: BTreeMap<PrKey, Reread>,
}

/// The state of one part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    /// The change of the newest run.
    expect: Expect,
    /// The data at the start of the first run, for the changes that
    /// compare with the start.
    baseline: Baseline,
    /// The runs of the part that have no result yet.
    running: u32,
    /// The start of the newest run.
    started: DateTime<Utc>,
    /// The success of the newest run. `None` while a run goes on.
    done_at: Option<DateTime<Utc>>,
    /// True when the part added its pull request to the announced pull
    /// requests of the notification. A failure removes it again.
    announced: bool,
}

/// The data at the start of a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Baseline {
    None,
    /// The comments of the thread or of the pull request.
    Comments(usize),
    /// The latest review of the viewer and the review request.
    Review(Option<ReviewState>, bool),
}

/// One run of an action.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Run {
    parts: Vec<Pending>,
    /// True when a PR view (`chaoshub pr`) runs the action and tells the
    /// list by the list socket.
    from_pane: bool,
}

/// The new reads of one pull request after a success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reread {
    /// The time of the last read, or of the success.
    last: DateTime<Utc>,
    /// The index of the next wait in `REREAD_WAITS`.
    step: usize,
}

/// The end of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunEnd {
    pub parts: Vec<Pending>,
    /// The pull requests whose announcement a failed part removes.
    pub unannounce: Vec<PrKey>,
}

/// A new read of a pull request that the map asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RereadReq {
    pub pr: PrKey,
    /// True when a part of the pull request waits for the conversation.
    pub detail: bool,
}

impl PendingMap {
    /// True when no part is in the map.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True while a part is in the map. The parts pulse, so each tick
    /// draws a new frame. A due read alone needs the tick, not a frame.
    pub(crate) fn has_parts(&self) -> bool {
        !self.entries.is_empty()
    }

    /// True while a part is in the map or a new read is due. The tick
    /// must then run.
    pub(crate) fn busy(&self) -> bool {
        !self.entries.is_empty() || !self.rereads.is_empty()
    }

    /// The start of the newest run of `part` of `pr` while the part is in
    /// the map.
    pub(crate) fn started(&self, pr: &PrKey, part: &PendingPart) -> Option<DateTime<Utc>> {
        self.entries
            .get(&(pr.clone(), part.clone()))
            .map(|entry| entry.started)
    }

    /// The start of the newest run of any part of `pr` while a part of
    /// `pr` is in the map.
    pub(crate) fn any_started(&self, pr: &PrKey) -> Option<DateTime<Utc>> {
        self.entries
            .iter()
            .filter(|((key, _), _)| key == pr)
            .map(|(_, entry)| entry.started)
            .max()
    }

    /// The review event of the pending review of `pr`, if any.
    pub(crate) fn review(&self, pr: &PrKey) -> Option<ReviewKind> {
        match self.entries.get(&(pr.clone(), PendingPart::Review))?.expect {
            Expect::Review(kind) => Some(kind),
            _ => None,
        }
    }

    /// The pull requests of the pending approvals. GitHub requests no
    /// review of you after an approval, so the Review tab hides them.
    pub(crate) fn approvals(&self) -> BTreeSet<PrKey> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.expect == Expect::Review(ReviewKind::Approve))
            .map(|((pr, _), _)| pr.clone())
            .collect()
    }

    /// The pending reactions on `pr`: the comment or review id, the kind,
    /// and true to add.
    pub(crate) fn reactions(&self, pr: &PrKey) -> Vec<(&str, ReactionKind, bool)> {
        self.entries
            .iter()
            .filter(|((key, _), _)| key == pr)
            .filter_map(|((_, part), entry)| match (part, entry.expect) {
                (PendingPart::Reaction(subject, kind), Expect::Reaction(add)) => {
                    Some((subject.as_str(), *kind, add))
                }
                _ => None,
            })
            .collect()
    }

    /// Moves every time of the map `by` into the past, so a test can
    /// reach a time limit.
    #[cfg(test)]
    pub(crate) fn age(&mut self, by: TimeDelta) {
        for entry in self.entries.values_mut() {
            entry.started -= by;
            entry.done_at = entry.done_at.map(|at| at - by);
        }
        for reread in self.rereads.values_mut() {
            reread.last -= by;
        }
    }

    /// Marks the pending review of `pr`: it added `pr` to the announced
    /// pull requests of the notification.
    pub(crate) fn set_announced(&mut self, pr: &PrKey) {
        if let Some(entry) = self.entries.get_mut(&(pr.clone(), PendingPart::Review)) {
            entry.announced = true;
        }
    }

    /// Starts a run of the action `id` on the pull request `target` at
    /// `now`, with its parts. The newest change of a part wins. False
    /// when the run has no part.
    pub(crate) fn start(
        &mut self,
        id: &'static str,
        target: PrKey,
        parts: Vec<Pending>,
        from_pane: bool,
        now: DateTime<Utc>,
        view: View<'_>,
    ) -> bool {
        if parts.is_empty() {
            return false;
        }
        for part in &parts {
            let key = (part.pr.clone(), part.part.clone());
            let entry = self.entries.entry(key).or_insert_with(|| Entry {
                expect: part.expect,
                baseline: baseline(part, view),
                running: 0,
                started: now,
                done_at: None,
                announced: false,
            });
            entry.expect = part.expect;
            entry.running += 1;
            entry.started = now;
            entry.done_at = None;
        }
        self.runs
            .entry((id, target))
            .or_default()
            .push_back(Run { parts, from_pane });
        true
    }

    /// Notes the parts of the next result of the action `id` on `target`
    /// (`ActionEvent::Parts`).
    pub(crate) fn result_parts(&mut self, id: &'static str, target: PrKey, parts: Vec<Pending>) {
        self.next.insert((id, target), parts);
    }

    /// The result of a run of the action `id` on `target`, at `now`: the
    /// run with the parts of `result_parts`, else the oldest run. A part
    /// of a success waits for its change. A part of a failure goes, when
    /// no other run of the part goes on. `None` without a run.
    pub(crate) fn done(
        &mut self,
        id: &'static str,
        target: &PrKey,
        ok: bool,
        now: DateTime<Utc>,
    ) -> Option<RunEnd> {
        let key = (id, target.clone());
        let parts = self.next.remove(&key);
        let queue = self.runs.get_mut(&key)?;
        let at = parts
            .and_then(|parts| queue.iter().position(|run| run.parts == parts))
            .unwrap_or(0);
        let run = queue.remove(at);
        if queue.is_empty() {
            self.runs.remove(&key);
        }
        let run = run?;
        let unannounce = self.finish(&run.parts, ok, now);
        Some(RunEnd {
            parts: run.parts,
            unannounce,
        })
    }

    /// The end of the PR view of `target`: the PR view cannot send the
    /// results of its runs any more. Each of its runs on `target` ends as
    /// a success, so the change or `SETTLE_LIMIT` ends its parts. True
    /// when a run ended.
    pub(crate) fn pane_ended(&mut self, target: &PrKey, now: DateTime<Utc>) -> bool {
        let mut ended = Vec::new();
        for ((_, pr), queue) in &mut self.runs {
            if pr == target {
                ended.extend(queue.iter().filter(|run| run.from_pane).cloned());
                queue.retain(|run| !run.from_pane);
            }
        }
        self.runs.retain(|_, queue| !queue.is_empty());
        for run in &ended {
            self.finish(&run.parts, true, now);
        }
        !ended.is_empty()
    }

    /// One run of each part of `parts` ended. Returns the pull requests
    /// whose announcement a failed part removes.
    fn finish(&mut self, parts: &[Pending], ok: bool, now: DateTime<Utc>) -> Vec<PrKey> {
        let mut unannounce = Vec::new();
        for part in parts {
            let key = (part.pr.clone(), part.part.clone());
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            entry.running = entry.running.saturating_sub(1);
            if entry.running > 0 {
                continue;
            }
            if ok {
                entry.done_at = Some(now);
                self.rereads
                    .insert(part.pr.clone(), Reread { last: now, step: 0 });
            } else if self.entries.remove(&key).is_some_and(|e| e.announced) {
                unannounce.push(part.pr.clone());
            }
        }
        unannounce
    }

    /// Removes the parts whose change shows in the GitHub data of `view`,
    /// and the parts over a time limit at `now`. True when a part went.
    pub(crate) fn sweep(&mut self, view: View<'_>, now: DateTime<Utc>) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let before = self.entries.len();
        self.entries
            .retain(|(pr, part), entry| match entry.done_at {
                None => entry.running == 0 || now - entry.started < RUN_LIMIT,
                Some(done_at) => now - done_at < SETTLE_LIMIT && !settled(pr, part, entry, view),
            });
        if self.entries.len() == before {
            return false;
        }
        // A run whose parts all went has nothing to show.
        let entries = &self.entries;
        let live = |part: &Pending| entries.contains_key(&(part.pr.clone(), part.part.clone()));
        for queue in self.runs.values_mut() {
            queue.retain(|run| run.parts.iter().any(live));
        }
        self.runs.retain(|_, queue| !queue.is_empty());
        true
    }

    /// The new reads that are due at `now`. A pull request gets no more
    /// reads when its parts settled and GitHub knows its merge state, or
    /// after the last wait of `REREAD_WAITS`.
    pub(crate) fn due_rereads(&mut self, view: View<'_>, now: DateTime<Utc>) -> Vec<RereadReq> {
        let entries = &self.entries;
        let pending = |pr: &PrKey| entries.keys().any(|(key, _)| key == pr);
        self.rereads.retain(|pr, reread| {
            reread.step < REREAD_WAITS.len() && (pending(pr) || unknown_merge_state(view, pr))
        });
        let mut due = Vec::new();
        for (pr, reread) in &mut self.rereads {
            if now - reread.last < REREAD_WAITS[reread.step] {
                continue;
            }
            reread.last = now;
            reread.step += 1;
            let detail = entries
                .keys()
                .any(|(key, part)| key == pr && detail_part(part));
            due.push(RereadReq {
                pr: pr.clone(),
                detail,
            });
        }
        due
    }
}

/// True when the conversation of the pull request shows `part`.
fn detail_part(part: &PendingPart) -> bool {
    matches!(
        part,
        PendingPart::Thread(_) | PendingPart::Comment | PendingPart::Reaction(..)
    )
}

/// True when `pr` is open and GitHub did not compute its merge state yet.
/// The data can then keep the last known state (`AppData::computing`).
fn unknown_merge_state(view: View<'_>, pr: &PrKey) -> bool {
    view.computing.contains_key(pr)
        || view.prs.get(pr).is_some_and(|pr| {
            let head = &pr.head.value;
            head.state == PrState::Open
                && (head.mergeable == Mergeable::Unknown
                    || head.merge_state == MergeStateStatus::Unknown)
        })
}

/// The data of `part` at its start, for the changes that compare with the
/// start.
fn baseline(part: &Pending, view: View<'_>) -> Baseline {
    match part.expect {
        Expect::NewComment => {
            let detail = view.details.get(&part.pr).map(|detail| &detail.value);
            Baseline::Comments(detail.map_or(0, |detail| comments(detail, &part.part)))
        }
        Expect::ReviewChanged => view.prs.get(&part.pr).map_or(Baseline::None, |pr| {
            Baseline::Review(pr.head.value.viewer_review, pr.head.value.review_requested)
        }),
        _ => Baseline::None,
    }
}

/// The comments of the thread of `part`, or of the pull request.
fn comments(detail: &PrDetail, part: &PendingPart) -> usize {
    match part {
        PendingPart::Thread(id) => detail
            .threads
            .iter()
            .find(|thread| thread.id == *id)
            .map_or(0, |thread| thread.comments.len()),
        _ => detail.comments.len(),
    }
}

/// The reactions of the comment or the review `id` in `detail`.
fn subject_reactions<'a>(detail: &'a PrDetail, id: &str) -> Option<&'a [Reaction]> {
    detail
        .comments
        .iter()
        .chain(detail.threads.iter().flat_map(|t| t.comments.iter()))
        .find(|comment| comment.id == id)
        .map(|comment| comment.reactions.as_slice())
        .or_else(|| {
            detail
                .reviews
                .iter()
                .find(|review| review.id == id)
                .map(|review| review.reactions.as_slice())
        })
}

/// True when the GitHub data of `view` shows the change of `part` of
/// `pr`. A pull request that is gone shows nothing, so its parts settle.
/// A conversation that is not loaded shows no change.
fn settled(pr: &PrKey, part: &PendingPart, entry: &Entry, view: View<'_>) -> bool {
    let Some(data) = view.prs.get(pr) else {
        return true;
    };
    let head = &data.head.value;
    let detail = view.details.get(pr).map(|detail| &detail.value);
    match entry.expect {
        Expect::Review(kind) => {
            let state = match kind {
                ReviewKind::Approve => ReviewState::Approved,
                ReviewKind::RequestChanges => ReviewState::ChangesRequested,
                ReviewKind::Comment => ReviewState::Commented,
            };
            head.viewer_review == Some(state)
                && (kind == ReviewKind::Comment || !head.review_requested)
        }
        Expect::ReviewChanged => {
            entry.baseline != Baseline::Review(head.viewer_review, head.review_requested)
        }
        Expect::Merged => head.state != PrState::Open || head.in_merge_queue,
        Expect::Closed => head.state == PrState::Closed,
        Expect::Draft(draft) => head.is_draft == draft,
        Expect::Rerun => crate::actions::rerun_plan(&data.checks.value).is_empty(),
        Expect::NewComment => match (detail, entry.baseline) {
            (Some(detail), Baseline::Comments(before)) => comments(detail, part) > before,
            _ => false,
        },
        Expect::Reaction(add) => {
            let PendingPart::Reaction(subject, kind) = part else {
                return true;
            };
            detail.is_some_and(|detail| {
                subject_reactions(detail, subject)
                    .is_none_or(|reactions| has_mine(reactions, *kind) == add)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testdata;
    use crate::domain::check::{Check, CheckConclusion, CheckKind, CheckStatus};
    use crate::domain::test_time;

    /// The data of #1 of `testdata` with a conversation.
    struct Data {
        prs: BTreeMap<PrKey, PullRequest>,
        details: BTreeMap<PrKey, Observed<PrDetail>>,
        computing: BTreeMap<PrKey, DateTime<Utc>>,
    }

    impl Data {
        fn new() -> Self {
            let pr = testdata::pr("chaoshub", 1, "One");
            let details = BTreeMap::from([(
                pr.key.clone(),
                testdata::observed(testdata::detail_with_reactions()),
            )]);
            Self {
                prs: BTreeMap::from([(pr.key.clone(), pr)]),
                details,
                computing: BTreeMap::new(),
            }
        }

        fn view(&self) -> View<'_> {
            View {
                prs: &self.prs,
                details: &self.details,
                computing: &self.computing,
            }
        }

        fn pr(&mut self) -> &mut PullRequest {
            self.prs.get_mut(&key()).unwrap()
        }

        fn detail(&mut self) -> &mut PrDetail {
            &mut self.details.get_mut(&key()).unwrap().value
        }
    }

    fn key() -> PrKey {
        testdata::pr_key("chaoshub", 1)
    }

    fn part(part: PendingPart, expect: Expect) -> Pending {
        Pending {
            pr: key(),
            part,
            expect,
        }
    }

    fn draft() -> Pending {
        part(PendingPart::Draft, Expect::Draft(true))
    }

    fn at(secs: i64) -> DateTime<Utc> {
        test_time() + TimeDelta::seconds(secs)
    }

    /// Starts one run of `parts` at 0 s and gives it a success at 1 s.
    fn succeed(map: &mut PendingMap, data: &Data, parts: Vec<Pending>) {
        assert!(map.start("x", key(), parts, false, at(0), data.view()));
        assert!(map.done("x", &key(), true, at(1)).is_some());
    }

    #[test]
    fn a_run_without_parts_adds_nothing() {
        let data = Data::new();
        let mut map = PendingMap::default();
        assert!(!map.start("x", key(), Vec::new(), false, at(0), data.view()));
        assert!(map.is_empty());
        assert_eq!(map.done("x", &key(), true, at(1)), None);
    }

    #[test]
    fn newer_data_without_the_change_keeps_the_part() {
        let mut data = Data::new();
        let mut map = PendingMap::default();
        assert!(map.start("x", key(), vec![draft()], false, at(0), data.view()));
        assert_eq!(map.started(&key(), &PendingPart::Draft), Some(at(0)));
        assert!(!map.sweep(data.view(), at(1)), "the run goes on");
        data.pr().head.value.is_draft = true;
        assert!(!map.sweep(data.view(), at(2)), "no result yet");
        data.pr().head.value.is_draft = false;
        map.done("x", &key(), true, at(3));
        data.pr().head.observed_at = at(4);
        assert!(!map.sweep(data.view(), at(4)), "an early read");
        assert!(map.started(&key(), &PendingPart::Draft).is_some());
        data.pr().head.value.is_draft = true;
        assert!(map.sweep(data.view(), at(5)), "the change shows");
        assert!(map.is_empty());
    }

    #[test]
    fn an_approval_settles_on_the_review_without_a_request() {
        let mut data = Data::new();
        let mut map = PendingMap::default();
        data.pr().head.value.review_requested = true;
        data.pr().head.value.viewer_review = Some(ReviewState::Approved);
        let approve = part(PendingPart::Review, Expect::Review(ReviewKind::Approve));
        succeed(&mut map, &data, vec![approve]);
        assert_eq!(map.approvals(), BTreeSet::from([key()]));
        assert_eq!(map.review(&key()), Some(ReviewKind::Approve));
        assert!(
            !map.sweep(data.view(), at(2)),
            "an old approval while GitHub still requests the review"
        );
        data.pr().head.value.review_requested = false;
        assert!(map.sweep(data.view(), at(3)));
        assert!(map.approvals().is_empty());
    }

    #[test]
    fn a_change_request_and_a_comment_review_settle_on_their_state() {
        let mut data = Data::new();
        let mut map = PendingMap::default();
        let changes = part(
            PendingPart::Review,
            Expect::Review(ReviewKind::RequestChanges),
        );
        succeed(&mut map, &data, vec![changes]);
        data.pr().head.value.viewer_review = Some(ReviewState::Approved);
        assert!(!map.sweep(data.view(), at(2)));
        data.pr().head.value.viewer_review = Some(ReviewState::ChangesRequested);
        assert!(map.sweep(data.view(), at(3)));

        let comment = part(PendingPart::Review, Expect::Review(ReviewKind::Comment));
        succeed(&mut map, &data, vec![comment]);
        data.pr().head.value.review_requested = true;
        assert!(!map.sweep(data.view(), at(2)));
        data.pr().head.value.viewer_review = Some(ReviewState::Commented);
        assert!(map.sweep(data.view(), at(3)), "a request can stay");
    }

    #[test]
    fn a_review_in_codeview_settles_on_any_review_change() {
        let mut data = Data::new();
        let mut map = PendingMap::default();
        data.pr().head.value.review_requested = true;
        succeed(
            &mut map,
            &data,
            vec![part(PendingPart::Review, Expect::ReviewChanged)],
        );
        assert!(!map.sweep(data.view(), at(2)));
        assert_eq!(map.review(&key()), None, "no known event");
        data.pr().head.value.review_requested = false;
        assert!(map.sweep(data.view(), at(3)));
    }

    #[test]
    fn a_merge_and_a_close_settle_on_the_state() {
        let mut data = Data::new();
        let mut map = PendingMap::default();
        succeed(
            &mut map,
            &data,
            vec![part(PendingPart::Merge, Expect::Merged)],
        );
        assert!(!map.sweep(data.view(), at(2)));
        data.pr().head.value.in_merge_queue = true;
        assert!(map.sweep(data.view(), at(3)), "the merge queue");
        data.pr().head.value.in_merge_queue = false;
        succeed(
            &mut map,
            &data,
            vec![part(PendingPart::Merge, Expect::Merged)],
        );
        data.pr().head.value.state = PrState::Merged;
        assert!(map.sweep(data.view(), at(2)));

        data.pr().head.value.state = PrState::Open;
        succeed(
            &mut map,
            &data,
            vec![part(PendingPart::State, Expect::Closed)],
        );
        assert!(!map.sweep(data.view(), at(2)));
        data.pr().head.value.state = PrState::Closed;
        assert!(map.sweep(data.view(), at(3)));
    }

    #[test]
    fn a_rerun_settles_when_no_failed_check_can_run_again() {
        let mut data = Data::new();
        let mut map = PendingMap::default();
        let check = |status, conclusion| Check {
            name: "ci".into(),
            kind: CheckKind::CheckRun,
            status,
            conclusion,
            started: None,
            completed: None,
            url: None,
            run_id: Some(7),
            suite_id: None,
        };
        data.pr().checks.value = vec![check(
            CheckStatus::Completed,
            Some(CheckConclusion::Failure),
        )];
        succeed(
            &mut map,
            &data,
            vec![part(PendingPart::Checks, Expect::Rerun)],
        );
        assert!(!map.sweep(data.view(), at(2)));
        data.pr().checks.value = vec![check(CheckStatus::Queued, None)];
        assert!(map.sweep(data.view(), at(3)));
    }

    #[test]
    fn a_reply_settles_on_a_new_comment() {
        let mut data = Data::new();
        let mut map = PendingMap::default();
        let thread = data.detail().threads[0].id.clone();
        succeed(
            &mut map,
            &data,
            vec![
                part(PendingPart::Thread(thread), Expect::NewComment),
                part(PendingPart::Comment, Expect::NewComment),
            ],
        );
        assert!(!map.sweep(data.view(), at(2)));
        let comment = data.detail().threads[0].comments[0].clone();
        data.detail().threads[0].comments.push(comment);
        assert!(map.sweep(data.view(), at(3)), "the thread reply");
        assert!(!map.is_empty(), "the comment on the pull request");
        let comment = data.detail().comments[0].clone();
        data.detail().comments.push(comment);
        assert!(map.sweep(data.view(), at(4)));
        assert!(map.is_empty());
    }

    #[test]
    fn a_reaction_settles_when_the_viewer_has_it() {
        let mut data = Data::new();
        let mut map = PendingMap::default();
        let subject = data.detail().comments[0].id.clone();
        let reaction = PendingPart::Reaction(subject.clone(), ReactionKind::Rocket);
        succeed(
            &mut map,
            &data,
            vec![part(reaction.clone(), Expect::Reaction(true))],
        );
        assert_eq!(
            map.reactions(&key()),
            [(subject.as_str(), ReactionKind::Rocket, true)]
        );
        assert!(!map.sweep(data.view(), at(2)));
        crate::domain::detail::set_mine(
            &mut data.detail().comments[0].reactions,
            ReactionKind::Rocket,
            true,
        );
        assert!(map.sweep(data.view(), at(3)));
        assert_eq!(map.reactions(&key()), []);
    }

    #[test]
    fn results_in_another_order_end_the_right_reaction() {
        let data = Data::new();
        let mut map = PendingMap::default();
        let reaction = |kind| {
            part(
                PendingPart::Reaction("C_1".into(), kind),
                Expect::Reaction(true),
            )
        };
        let heart = reaction(ReactionKind::Heart);
        let rocket = reaction(ReactionKind::Rocket);
        map.start(
            "react",
            key(),
            vec![heart.clone()],
            false,
            at(0),
            data.view(),
        );
        map.start(
            "react",
            key(),
            vec![rocket.clone()],
            false,
            at(0),
            data.view(),
        );
        map.result_parts("react", key(), vec![rocket]);
        let end = map.done("react", &key(), false, at(1)).unwrap();
        assert_eq!(end.parts, [reaction(ReactionKind::Rocket)]);
        assert_eq!(
            map.reactions(&key()),
            [("C_1", ReactionKind::Heart, true)],
            "the failed rocket rolls back, the heart stays"
        );
        let end = map.done("react", &key(), true, at(2)).unwrap();
        assert_eq!(end.parts, [heart]);
    }

    #[test]
    fn the_newest_change_of_a_part_wins() {
        let data = Data::new();
        let mut map = PendingMap::default();
        let reaction = |add| {
            part(
                PendingPart::Reaction("C_1".into(), ReactionKind::Heart),
                Expect::Reaction(add),
            )
        };
        map.start(
            "react",
            key(),
            vec![reaction(true)],
            false,
            at(0),
            data.view(),
        );
        map.start(
            "react",
            key(),
            vec![reaction(false)],
            false,
            at(1),
            data.view(),
        );
        assert_eq!(map.reactions(&key()), [("C_1", ReactionKind::Heart, false)]);
        map.done("react", &key(), false, at(2));
        assert!(!map.is_empty(), "the second run goes on");
        map.done("react", &key(), false, at(3));
        assert!(map.is_empty());
    }

    #[test]
    fn a_failure_removes_the_part_and_its_announcement() {
        let data = Data::new();
        let mut map = PendingMap::default();
        let approve = part(PendingPart::Review, Expect::Review(ReviewKind::Approve));
        map.start("x", key(), vec![approve], false, at(0), data.view());
        map.set_announced(&key());
        let end = map.done("x", &key(), false, at(1)).unwrap();
        assert_eq!(end.unannounce, [key()]);
        assert!(map.is_empty());
        assert!(!map.busy(), "no new reads after a failure");
    }

    #[test]
    fn the_time_limits_end_every_part() {
        let data = Data::new();
        let mut map = PendingMap::default();
        succeed(&mut map, &data, vec![draft()]);
        let limit = at(1) + SETTLE_LIMIT;
        assert!(!map.sweep(data.view(), limit - TimeDelta::seconds(1)));
        assert!(map.sweep(data.view(), limit), "no change in the data");
        assert!(map.is_empty());

        map.start("x", key(), vec![draft()], false, at(100), data.view());
        let limit = at(100) + RUN_LIMIT;
        assert!(!map.sweep(data.view(), limit - TimeDelta::seconds(1)));
        assert!(map.sweep(data.view(), limit), "no result");
        assert!(map.is_empty());
        assert_eq!(
            map.done("x", &key(), true, at(5000)),
            None,
            "the run went too"
        );
    }

    #[test]
    fn the_end_of_the_pr_view_ends_its_runs() {
        let data = Data::new();
        let mut map = PendingMap::default();
        map.start("x", key(), vec![draft()], true, at(0), data.view());
        let close = part(PendingPart::State, Expect::Closed);
        map.start("close", key(), vec![close], false, at(0), data.view());
        assert!(map.pane_ended(&key(), at(5)));
        assert!(!map.pane_ended(&key(), at(6)), "only once");
        assert!(map.started(&key(), &PendingPart::Draft).is_some());
        assert!(map.sweep(data.view(), at(5) + SETTLE_LIMIT));
        assert!(map.started(&key(), &PendingPart::Draft).is_none());
        assert!(
            map.started(&key(), &PendingPart::State).is_some(),
            "a run of the list"
        );
    }

    #[test]
    fn new_reads_back_off_and_stop_when_settled_and_known() {
        let mut data = Data::new();
        let mut map = PendingMap::default();
        data.pr().head.value.mergeable = Mergeable::Unknown;
        data.pr().head.value.merge_state = MergeStateStatus::Unknown;
        succeed(&mut map, &data, vec![draft()]);
        assert!(map.busy());
        assert_eq!(map.due_rereads(data.view(), at(2)), []);
        let first = map.due_rereads(data.view(), at(3));
        assert_eq!(
            first,
            [RereadReq {
                pr: key(),
                detail: false
            }],
            "2 s after the success"
        );
        assert_eq!(map.due_rereads(data.view(), at(7)), [], "then 5 s");
        assert_eq!(map.due_rereads(data.view(), at(8)).len(), 1);
        data.pr().head.value.is_draft = true;
        assert!(map.sweep(data.view(), at(9)), "settled");
        assert_eq!(
            map.due_rereads(data.view(), at(23)).len(),
            1,
            "the merge state is not known yet"
        );
        data.pr().head.value.mergeable = Mergeable::Mergeable;
        data.pr().head.value.merge_state = MergeStateStatus::Clean;
        assert_eq!(map.due_rereads(data.view(), at(60)), []);
        assert!(!map.busy(), "settled and known");
    }

    #[test]
    fn new_reads_stop_after_the_last_wait() {
        let data = Data::new();
        let mut map = PendingMap::default();
        let thread = part(PendingPart::Thread("T_9".into()), Expect::NewComment);
        succeed(&mut map, &data, vec![thread]);
        let mut now = at(1);
        let mut reads = Vec::new();
        for _ in 0..200 {
            now += TimeDelta::seconds(1);
            reads.extend(map.due_rereads(data.view(), now));
        }
        assert_eq!(reads.len(), REREAD_WAITS.len());
        assert!(
            reads.iter().all(|read| read.detail),
            "a reply needs the detail"
        );
        assert!(!map.rereads.contains_key(&key()));
    }
}
