//! The agent runs and their result files.
//!
//! `run` starts the backend of `[agent] backend` for one tool on one pull
//! request: pr-agent in `run`, or Open Code Review (`ocr`) in `ocr`.
//! `store` keeps the result of each tool in a JSON file under
//! `<state_dir>/agent/<owner>/<repo>/<number>/`. codeview reads the same
//! files. pr-agent runs with `publish_output=false`, and ocr never writes
//! to GitHub, so a run writes nothing to the pull request. The tools and
//! the file content are plain data in `domain::agent`.

pub(crate) mod ocr;
pub(crate) mod run;
pub(crate) mod store;

/// The action id of the progress events of a run.
pub(crate) const ID: &str = "agent";
