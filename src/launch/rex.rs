//! The Rex launcher: split, zoom, wait, inspect, focus and close.
//!
//! Every command is an argv vector, never a shell string.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Output;

use futures::future::BoxFuture;
use serde_json::Value;
use tokio::process::Command;

use super::error::LaunchError;
use super::{LaunchHandle, Launcher};
use crate::bus::LaunchRequest;

/// The default direction of a new pane.
pub(crate) const DEFAULT_SPLIT: &str = "right";

/// Starts panes in Rex, next to the block of the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RexLauncher {
    /// The rex binary.
    pub bin: PathBuf,
    /// The block of the TUI, the anchor of every split.
    pub block: String,
    /// The session of the TUI. `None` leaves out `-s`, so Rex uses its
    /// current session.
    pub session: Option<String>,
    /// The direction of a new pane: right, left, above or below.
    pub split: String,
    /// The percent of the split that a new pane gets. `None` keeps the
    /// Rex default.
    pub ratio: Option<u8>,
}

impl RexLauncher {
    /// The launcher of the Rex pane that runs chaoshub, from `REX_BLOCK`,
    /// `REX_SESSION` and `REX_BIN_DIR`. `None` outside Rex, that is without
    /// `REX_BLOCK`. Without `REX_SESSION` the launcher has no session.
    pub(crate) fn from_env() -> Option<Self> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// The launcher from the variables that `var` gives. The binary is
    /// `$REX_BIN_DIR/rex` when that file exists, else `rex` on the path.
    fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let text = |name: &str| {
            var(name)
                .and_then(|value| value.into_string().ok())
                .filter(|value| !value.is_empty())
        };
        let block = text("REX_BLOCK")?;
        let session = text("REX_SESSION");
        let bin = var("REX_BIN_DIR")
            .map(|dir| PathBuf::from(dir).join("rex"))
            .filter(|bin| bin.is_file())
            .unwrap_or_else(|| PathBuf::from("rex"));
        Some(Self {
            bin,
            block,
            session,
            split: DEFAULT_SPLIT.to_owned(),
            ratio: None,
        })
    }

    /// `rex split` of `req` beside the TUI block, with a JSON result. The
    /// split and the ratio of the request win over the ones of the
    /// launcher. The env of the request goes through `env NAME=VALUE`.
    fn split_args(&self, req: &LaunchRequest) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["split".into(), "-b".into(), (&self.block).into()];
        self.push_session(&mut args);
        let split = req.split.as_ref().unwrap_or(&self.split);
        args.push(format!("--split={split}").into());
        if let Some(ratio) = req.ratio.or(self.ratio) {
            args.extend(["--ratio".into(), ratio.to_string().into()]);
        }
        args.extend([
            "-c".into(),
            req.cwd.clone().into(),
            "--label".into(),
            (&req.label).into(),
            "--json".into(),
            "--".into(),
        ]);
        if !req.env.is_empty() {
            args.push("env".into());
            args.extend(
                req.env
                    .iter()
                    .map(|(name, value)| format!("{name}={value}").into()),
            );
        }
        args.extend(req.argv.iter().map(Into::into));
        args
    }

    /// `rex zoom` that makes the block fill its window.
    fn zoom_args(block_id: &str) -> Vec<OsString> {
        vec!["zoom".into(), "-b".into(), block_id.into(), "--on".into()]
    }

    /// `rex wait` for the command of the pane.
    fn wait_args(&self, handle: &LaunchHandle) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["wait".into(), (&handle.label).into()];
        self.push_session(&mut args);
        args
    }

    fn inspect_args(&self, handle: &LaunchHandle) -> Vec<OsString> {
        self.block_args(&["block", "inspect"], handle)
    }

    fn focus_args(&self, handle: &LaunchHandle) -> Vec<OsString> {
        self.block_args(&["focus"], handle)
    }

    fn close_args(&self, handle: &LaunchHandle) -> Vec<OsString> {
        self.block_args(&["block", "close"], handle)
    }

    /// `rex <command> <block id> [-s <session>]`.
    fn block_args(&self, command: &[&str], handle: &LaunchHandle) -> Vec<OsString> {
        let mut args: Vec<OsString> = command.iter().map(Into::into).collect();
        args.push((&handle.block_id).into());
        self.push_session(&mut args);
        args
    }

    /// Adds `-s <session>` when the launcher has a session.
    fn push_session(&self, args: &mut Vec<OsString>) {
        if let Some(session) = &self.session {
            args.extend(["-s".into(), session.into()]);
        }
    }

    fn command(&self, args: Vec<OsString>) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args);
        cmd
    }

    /// Runs `cmd` to its end and returns its output.
    async fn output(&self, mut cmd: Command) -> Result<Output, LaunchError> {
        cmd.output().await.map_err(|source| LaunchError::Spawn {
            program: self.bin.display().to_string(),
            source,
        })
    }

    /// Runs the rex `command` with `args`. An exit status other than zero
    /// is an error with the standard error text.
    async fn run(&self, command: &'static str, args: Vec<OsString>) -> Result<Output, LaunchError> {
        let output = self.output(self.command(args)).await?;
        if !output.status.success() {
            return Err(LaunchError::Failed {
                command,
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(output)
    }
}

/// The block id from the JSON result of `rex split`. Rex prints
/// `{ "anchor_id": "block:…", "block_ids": ["block:…"], "split_ids":
/// ["split:…"], "revision": 48 }`, so the first entry of `block_ids` comes
/// first. Older field names follow, then `label`, which Rex also accepts
/// as a target.
fn parse_block_id(stdout: &str, label: &str) -> String {
    serde_json::from_str::<Value>(stdout.trim())
        .ok()
        .and_then(|value| {
            [
                "/block_ids/0",
                "/block_id",
                "/block/block_id",
                "/block/id",
                "/id",
            ]
            .iter()
            .find_map(|pointer| {
                value
                    .pointer(pointer)
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(ToOwned::to_owned)
            })
        })
        .unwrap_or_else(|| label.to_owned())
}

impl Launcher for RexLauncher {
    fn launch(&self, req: &LaunchRequest) -> BoxFuture<'_, Result<LaunchHandle, LaunchError>> {
        let args = self.split_args(req);
        let label = req.label.clone();
        let zoom = req.zoom;
        Box::pin(async move {
            let output = self.run("split", args).await?;
            let block_id = parse_block_id(&String::from_utf8_lossy(&output.stdout), &label);
            if zoom && let Err(err) = self.run("zoom", Self::zoom_args(&block_id)).await {
                tracing::warn!(%err, %block_id, "rex zoom failed, the pane stays unzoomed");
            }
            Ok(LaunchHandle { label, block_id })
        })
    }

    fn wait(&self, handle: &LaunchHandle) -> BoxFuture<'_, Result<i32, LaunchError>> {
        let mut cmd = self.command(self.wait_args(handle));
        // A cancelled wait must not leave the rex process behind.
        cmd.kill_on_drop(true);
        Box::pin(async move {
            let output = self.output(cmd).await?;
            Ok(output.status.code().unwrap_or(-1))
        })
    }

    fn alive(&self, handle: &LaunchHandle) -> BoxFuture<'_, bool> {
        let args = self.inspect_args(handle);
        Box::pin(async move { self.run("block inspect", args).await.is_ok() })
    }

    fn focus(&self, handle: &LaunchHandle) -> BoxFuture<'_, Result<(), LaunchError>> {
        let args = self.focus_args(handle);
        Box::pin(async move { self.run("focus", args).await.map(drop) })
    }

    fn close(&self, handle: &LaunchHandle) -> BoxFuture<'_, Result<(), LaunchError>> {
        let args = self.close_args(handle);
        Box::pin(async move { self.run("block close", args).await.map(drop) })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use pretty_assertions::assert_eq;

    use super::*;

    fn launcher() -> RexLauncher {
        RexLauncher {
            bin: PathBuf::from("/Applications/Rex Server.app/rex"),
            block: "block:01".into(),
            session: Some("session:01".into()),
            split: DEFAULT_SPLIT.into(),
            ratio: None,
        }
    }

    fn request() -> LaunchRequest {
        LaunchRequest {
            argv: vec![
                "nvim".into(),
                "-c".into(),
                "autocmd User CodeViewSessionClosed ++once qall".into(),
                "-c".into(),
                "Codeview pr 7".into(),
            ],
            cwd: PathBuf::from("/home/me/code/my repo"),
            env: vec![],
            label: "chaoshub-review-chaosteil-chaoshub-7-a1b2".into(),
            zoom: true,
            split: None,
            ratio: None,
        }
    }

    fn handle() -> LaunchHandle {
        LaunchHandle {
            label: "chaoshub-review-chaosteil-chaoshub-7-a1b2".into(),
            block_id: "block:02".into(),
        }
    }

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn split_argv() {
        assert_eq!(
            strings(&launcher().split_args(&request())),
            [
                "split",
                "-b",
                "block:01",
                "-s",
                "session:01",
                "--split=right",
                "-c",
                "/home/me/code/my repo",
                "--label",
                "chaoshub-review-chaosteil-chaoshub-7-a1b2",
                "--json",
                "--",
                "nvim",
                "-c",
                "autocmd User CodeViewSessionClosed ++once qall",
                "-c",
                "Codeview pr 7",
            ]
        );
    }

    #[test]
    fn argv_without_a_session_has_no_s_flag() {
        let rex = RexLauncher {
            session: None,
            ..launcher()
        };
        assert_eq!(
            strings(&rex.split_args(&request()))[..6],
            [
                "split",
                "-b",
                "block:01",
                "--split=right",
                "-c",
                "/home/me/code/my repo"
            ]
        );
        assert_eq!(
            strings(&rex.wait_args(&handle())),
            ["wait", "chaoshub-review-chaosteil-chaoshub-7-a1b2"]
        );
        assert_eq!(
            strings(&rex.inspect_args(&handle())),
            ["block", "inspect", "block:02"]
        );
        assert_eq!(strings(&rex.focus_args(&handle())), ["focus", "block:02"]);
        assert_eq!(
            strings(&rex.close_args(&handle())),
            ["block", "close", "block:02"]
        );
    }

    #[test]
    fn split_argv_with_ratio_direction_and_env() {
        let rex = RexLauncher {
            split: "below".into(),
            ratio: Some(70),
            ..launcher()
        };
        let req = LaunchRequest {
            argv: vec!["pr-agent".into(), "review".into()],
            env: vec![("OPENAI_KEY".into(), "a b".into())],
            ..request()
        };
        assert_eq!(
            strings(&rex.split_args(&req)),
            [
                "split",
                "-b",
                "block:01",
                "-s",
                "session:01",
                "--split=below",
                "--ratio",
                "70",
                "-c",
                "/home/me/code/my repo",
                "--label",
                "chaoshub-review-chaosteil-chaoshub-7-a1b2",
                "--json",
                "--",
                "env",
                "OPENAI_KEY=a b",
                "pr-agent",
                "review",
            ]
        );
    }

    #[test]
    fn the_split_and_the_ratio_of_the_request_win() {
        let rex = RexLauncher {
            split: "below".into(),
            ratio: Some(70),
            ..launcher()
        };
        let req = LaunchRequest {
            split: Some("left".into()),
            ratio: Some(40),
            ..request()
        };
        assert_eq!(
            strings(&rex.split_args(&req))[5..8],
            ["--split=left", "--ratio", "40"]
        );
    }

    #[test]
    fn a_request_without_a_split_and_a_ratio_uses_the_launcher() {
        let rex = RexLauncher {
            split: "below".into(),
            ratio: Some(70),
            ..launcher()
        };
        assert_eq!(
            strings(&rex.split_args(&request()))[5..8],
            ["--split=below", "--ratio", "70"]
        );
    }

    #[test]
    fn pane_argv() {
        let rex = launcher();
        assert_eq!(
            strings(&RexLauncher::zoom_args("block:02")),
            ["zoom", "-b", "block:02", "--on"]
        );
        assert_eq!(
            strings(&rex.wait_args(&handle())),
            [
                "wait",
                "chaoshub-review-chaosteil-chaoshub-7-a1b2",
                "-s",
                "session:01"
            ]
        );
        assert_eq!(
            strings(&rex.inspect_args(&handle())),
            ["block", "inspect", "block:02", "-s", "session:01"]
        );
        assert_eq!(
            strings(&rex.focus_args(&handle())),
            ["focus", "block:02", "-s", "session:01"]
        );
        assert_eq!(
            strings(&rex.close_args(&handle())),
            ["block", "close", "block:02", "-s", "session:01"]
        );
    }

    #[test]
    fn command_uses_the_binary_path() {
        let rex = launcher();
        let cmd = rex.command(rex.focus_args(&handle()));
        assert_eq!(
            cmd.as_std().get_program(),
            "/Applications/Rex Server.app/rex"
        );
    }

    #[test]
    fn block_id_from_json() {
        let label = "the-label";
        for (json, id) in [
            (
                r#"{"anchor_id":"block:01","block_ids":["block:02","block:09"],"split_ids":["split:01"],"revision":48}"#,
                "block:02",
            ),
            (
                r#"{"block_ids":["block:08"],"block_id":"block:03"}"#,
                "block:08",
            ),
            (r#"{"block_ids":[],"block_id":"block:03"}"#, "block:03"),
            (r#"{"block_id":"block:03"}"#, "block:03"),
            (
                r#"{"session_id":"session:01","block":{"block_id":"block:04","label":"x"}}"#,
                "block:04",
            ),
            (r#"{"block":{"id":"block:05"}}"#, "block:05"),
            (r#"{"id":"block:06"}"#, "block:06"),
            ("{\"block_id\":\"block:07\"}\n", "block:07"),
        ] {
            assert_eq!(parse_block_id(json, label), id, "{json}");
        }
    }

    #[test]
    fn block_id_falls_back_to_the_label() {
        for json in [
            "",
            "not json",
            "{}",
            r#"{"anchor_id":"block:01"}"#,
            r#"{"block_ids":[]}"#,
            r#"{"block_ids":[""]}"#,
            r#"{"block_id":""}"#,
            r#"{"block_id":7}"#,
        ] {
            assert_eq!(parse_block_id(json, "the-label"), "the-label", "{json}");
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).into()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn from_vars_inside_rex() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rex"), "").unwrap();
        let rex = RexLauncher::from_vars(vars(&[
            ("REX_BLOCK", "block:01"),
            ("REX_SESSION", "session:01"),
            ("REX_BIN_DIR", dir.path().to_str().unwrap()),
        ]))
        .unwrap();
        assert_eq!(
            rex,
            RexLauncher {
                bin: dir.path().join("rex"),
                block: "block:01".into(),
                session: Some("session:01".into()),
                split: "right".into(),
                ratio: None,
            }
        );
    }

    #[test]
    fn from_vars_falls_back_to_rex_on_the_path() {
        let rex = RexLauncher::from_vars(vars(&[
            ("REX_BLOCK", "block:01"),
            ("REX_SESSION", "session:01"),
            ("REX_BIN_DIR", "/no/such/dir"),
        ]))
        .unwrap();
        assert_eq!(rex.bin, Path::new("rex"));
        let rex =
            RexLauncher::from_vars(vars(&[("REX_BLOCK", "b"), ("REX_SESSION", "s")])).unwrap();
        assert_eq!(rex.bin, Path::new("rex"));
    }

    #[test]
    fn from_vars_without_a_session() {
        for pairs in [
            &[("REX_BLOCK", "block:01")][..],
            &[("REX_BLOCK", "block:01"), ("REX_SESSION", "")][..],
        ] {
            let rex = RexLauncher::from_vars(vars(pairs)).unwrap();
            assert_eq!(rex.block, "block:01");
            assert_eq!(rex.session, None);
        }
    }

    #[test]
    fn from_vars_outside_rex() {
        assert_eq!(RexLauncher::from_vars(vars(&[])), None);
        assert_eq!(
            RexLauncher::from_vars(vars(&[("REX_SESSION", "session:01")])),
            None
        );
        assert_eq!(
            RexLauncher::from_vars(vars(&[("REX_BLOCK", ""), ("REX_SESSION", "s")])),
            None
        );
    }

    #[test]
    fn rex_launcher_is_a_dyn_launcher() {
        let launcher: Box<dyn Launcher> = Box::new(launcher());
        drop(launcher);
    }
}
