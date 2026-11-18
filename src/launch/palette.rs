//! The command palette of Rex: one question with `rex.ui`.
//!
//! `rex.ui.input` and `rex.ui.pick` work only in the run function of an
//! action of the `init.lua` of the Rex server. An action of a `rex do`
//! script gets `unsupported`. Thus `contrib/rex/chaoshub.lua` registers
//! the action `chaoshub_ask`, and chaoshub runs it with
//! `rex do chaoshub_ask --args - --json`. The question goes to the action
//! as JSON on stdin. Without the action, chaoshub opens its own popups.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;

use futures::future::BoxFuture;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::bus::{PaletteAsk, PaletteReply};

/// The name of the action of `contrib/rex/chaoshub.lua` that asks the
/// question.
pub(crate) const ACTION: &str = "chaoshub_ask";
/// The first control API revision of Rex with `rex.ui`.
pub(crate) const MIN_API_REVISION: u64 = 17;
/// How long Rex shows the question, in seconds. It is the default of
/// `rex.ui`.
pub(crate) const TIMEOUT_SECS: u64 = 300;
/// The extra time of `rex do --timeout` after the question time. The
/// question ends first, so `rex do` gives the `timeout` reason.
const WAIT_MARGIN_SECS: u64 = 30;

/// Asks one question in the command palette of Rex. The method returns
/// a boxed future, so the trait stays usable as `dyn Palette`. A drop of
/// the future stops the question.
pub(crate) trait Palette: Send + Sync {
    /// Asks `ask` and waits for the reply.
    fn ask(&self, ask: &PaletteAsk) -> BoxFuture<'_, PaletteReply>;
}

/// The palette of the Rex server of the `rex` binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RexPalette {
    /// The rex binary.
    pub bin: PathBuf,
}

impl RexPalette {
    /// The palette of `bin`.
    pub(crate) fn new(bin: PathBuf) -> Self {
        Self { bin }
    }

    /// True when Rex can ask: the server has control API revision 17 or
    /// later, the server has the action `chaoshub_ask`, and at least one
    /// Rex app is connected. Each failure is false.
    pub(crate) async fn detect(&self) -> bool {
        let version = self.stdout(&["api", "call", "server.version"]).await;
        let Some(revision) = version.as_deref().and_then(api_revision) else {
            tracing::debug!("no api revision from rex, the palette stays off");
            return false;
        };
        if revision < MIN_API_REVISION {
            tracing::debug!(revision, "rex is too old for rex.ui");
            return false;
        }
        let actions = self.stdout(&["actions", "--json"]).await;
        if !actions.as_deref().is_some_and(has_ask_action) {
            tracing::debug!(
                action = ACTION,
                "the Rex server has no chaoshub action (contrib/rex/chaoshub.lua), the palette stays off"
            );
            return false;
        }
        let clients = self.stdout(&["client", "ls", "--json"]).await;
        let app = clients.as_deref().is_some_and(has_app_client);
        if !app {
            tracing::debug!("no Rex app is connected, the palette stays off");
        }
        app
    }

    /// The standard output of `rex <args>`. `None` when rex does not
    /// start or exits with a status other than zero.
    async fn stdout(&self, args: &[&str]) -> Option<String> {
        let output = Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await;
        match output {
            Ok(output) if output.status.success() => {
                Some(String::from_utf8_lossy(&output.stdout).into_owned())
            }
            Ok(output) => {
                tracing::debug!(
                    ?args,
                    status = %output.status,
                    stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                    "rex failed"
                );
                None
            }
            Err(err) => {
                tracing::debug!(%err, ?args, "cannot start rex");
                None
            }
        }
    }

    /// The arguments of `rex do` for the action.
    fn ask_args() -> Vec<OsString> {
        vec![
            "--timeout".into(),
            format!("{}s", TIMEOUT_SECS + WAIT_MARGIN_SECS).into(),
            "do".into(),
            ACTION.into(),
            "--args".into(),
            "-".into(),
            "--json".into(),
        ]
    }

    /// Runs the action with `ask` and returns the reply.
    async fn run(&self, ask: &PaletteAsk) -> Result<PaletteReply, String> {
        let input = json!({ "ask": ask, "timeout": TIMEOUT_SECS }).to_string();
        let mut child = Command::new(&self.bin)
            .args(Self::ask_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A dropped question must not leave the rex process behind.
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| format!("cannot start {}: {err}", self.bin.display()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.as_bytes())
                .await
                .map_err(|err| format!("cannot write to rex: {err}"))?;
        }
        let output = child
            .wait_with_output()
            .await
            .map_err(|err| format!("cannot wait for rex: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "rex do failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(parse_reply(&String::from_utf8_lossy(&output.stdout)))
    }
}

impl Palette for RexPalette {
    fn ask(&self, ask: &PaletteAsk) -> BoxFuture<'_, PaletteReply> {
        let ask = ask.clone();
        Box::pin(async move {
            self.run(&ask).await.unwrap_or_else(|err| {
                tracing::warn!(%err, "cannot ask in the Rex palette");
                PaletteReply::Fallback(err)
            })
        })
    }
}

/// The `api_revision` of the JSON of `rex api call server.version`.
fn api_revision(stdout: &str) -> Option<u64> {
    serde_json::from_str::<Value>(stdout.trim())
        .ok()?
        .get("api_revision")?
        .as_u64()
}

/// True when the JSON of `rex actions --json` has the action
/// `chaoshub_ask`.
fn has_ask_action(stdout: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) else {
        return false;
    };
    value
        .get("actions")
        .and_then(Value::as_array)
        .is_some_and(|actions| {
            actions
                .iter()
                .any(|action| action.get("name").and_then(Value::as_str) == Some(ACTION))
        })
}

/// True when the JSON of `rex client ls --json` has a connected client of
/// the kind `app`. A client without a connection state counts as
/// connected.
fn has_app_client(stdout: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) else {
        return false;
    };
    value
        .get("clients")
        .and_then(Value::as_array)
        .is_some_and(|clients| {
            clients.iter().any(|client| {
                client.pointer("/info/kind").and_then(Value::as_str) == Some("app")
                    && client
                        .get("connection_state")
                        .and_then(Value::as_str)
                        .is_none_or(|state| state == "connected")
            })
        })
}

/// The reply of the JSON result of the action: `{"answer": "..."}` or
/// `{"reason": "..."}`. `cancelled` and `timeout` are a cancel. Every
/// other reason, and a result that does not parse, is a fallback.
fn parse_reply(stdout: &str) -> PaletteReply {
    let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) else {
        return PaletteReply::Fallback(format!("rex do gave no JSON: {}", stdout.trim()));
    };
    if let Some(answer) = value.get("answer").and_then(Value::as_str) {
        return PaletteReply::Answer(answer.to_owned());
    }
    match value.get("reason").and_then(Value::as_str) {
        Some("cancelled" | "timeout") => PaletteReply::Cancelled,
        Some(reason) => PaletteReply::Fallback(reason.to_owned()),
        None => PaletteReply::Fallback(format!("rex do gave no answer: {}", stdout.trim())),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::bus::PaletteItem;

    fn pick() -> PaletteAsk {
        PaletteAsk::Pick {
            title: "author".into(),
            placeholder: String::new(),
            items: vec![PaletteItem {
                id: "alice".into(),
                title: "alice".into(),
                subtitle: None,
                detail: Some("current".into()),
                icon: Some("person"),
            }],
        }
    }

    #[test]
    fn the_ask_is_the_json_of_rex_ui() {
        let input = json!({ "ask": pick(), "timeout": 300 });
        assert_eq!(
            input,
            json!({
                "ask": {
                    "kind": "pick",
                    "title": "author",
                    "items": [{
                        "id": "alice",
                        "title": "alice",
                        "detail": "current",
                        "icon": "person",
                    }],
                },
                "timeout": 300,
            })
        );
        let ask = PaletteAsk::Input {
            title: "ask pr-agent about #7".into(),
            placeholder: "a question".into(),
        };
        assert_eq!(
            serde_json::to_value(ask).unwrap(),
            json!({ "kind": "input", "title": "ask pr-agent about #7", "placeholder": "a question" })
        );
    }

    const ARGV: [&str; 7] = [
        "--timeout",
        "330s",
        "do",
        "chaoshub_ask",
        "--args",
        "-",
        "--json",
    ];

    #[test]
    fn ask_argv() {
        let args: Vec<String> = RexPalette::ask_args()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ARGV);
    }

    #[test]
    fn replies_from_json() {
        for (stdout, reply) in [
            (
                "{\"answer\": \"alice\"}\n",
                PaletteReply::Answer("alice".into()),
            ),
            ("{\"answer\": \"\"}", PaletteReply::Answer(String::new())),
            ("{\"reason\": \"cancelled\"}", PaletteReply::Cancelled),
            ("{\"reason\": \"timeout\"}", PaletteReply::Cancelled),
            (
                "{\"reason\": \"no_client\"}",
                PaletteReply::Fallback("no_client".into()),
            ),
            (
                "{\"reason\": \"unsupported\"}",
                PaletteReply::Fallback("unsupported".into()),
            ),
            (
                "{\"reason\": \"busy\"}",
                PaletteReply::Fallback("busy".into()),
            ),
            (
                "{\"reason\": \"failed\"}",
                PaletteReply::Fallback("failed".into()),
            ),
        ] {
            assert_eq!(parse_reply(stdout), reply, "{stdout}");
        }
        for stdout in ["", "not json", "{}", "{\"answer\": 7}"] {
            assert!(
                matches!(parse_reply(stdout), PaletteReply::Fallback(_)),
                "{stdout}"
            );
        }
    }

    #[test]
    fn api_revision_from_json() {
        let version = r#"{"rex_version":"0.0.1794893356-gfa0782","platform":"linux_amd64","api_revision":17}"#;
        assert_eq!(api_revision(version), Some(17));
        assert_eq!(api_revision("{\"rex_version\":\"x\"}"), None);
        assert_eq!(api_revision("not json"), None);
    }

    #[test]
    fn the_ask_action_from_json() {
        let actions = r#"{"actions":[
            {"name":"com.superlogical.terminal.reset","kind":"block"},
            {"name":"chaoshub_ask","args":{"type":"object"}}
        ]}"#;
        assert!(has_ask_action(actions));
        for json in [
            r#"{"actions":[{"name":"chaoshub_open_pr"}]}"#,
            r#"{"actions":[]}"#,
            "{}",
            "not json",
        ] {
            assert!(!has_ask_action(json), "{json}");
        }
    }

    #[test]
    fn app_clients_from_json() {
        let app = r#"{"clients":[
            {"client_id":"client:01","info":{"kind":"cli","name":"rex"},"connection_state":"connected"},
            {"client_id":"client:02","info":{"kind":"app","name":"Rex"},"connection_state":"connected"}
        ]}"#;
        assert!(has_app_client(app));
        let no_state = r#"{"clients":[{"info":{"kind":"app"}}]}"#;
        assert!(has_app_client(no_state));
        for json in [
            r#"{"clients":[{"info":{"kind":"cli"},"connection_state":"connected"}]}"#,
            r#"{"clients":[{"info":{"kind":"app"},"connection_state":"disconnected"}]}"#,
            r#"{"clients":[]}"#,
            "{}",
            "not json",
        ] {
            assert!(!has_app_client(json), "{json}");
        }
    }

    /// Writes `script` as an executable shell script to `bin`.
    fn write_bin(bin: &Path, script: &str) {
        std::fs::write(bin, script).unwrap();
        std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A fake `rex` in `dir`: a shell script that saves its arguments and
    /// its stdin, and prints `stdout` with the exit status `status`.
    fn fake_rex(dir: &Path, stdout: &str, status: i32) -> PathBuf {
        let bin = dir.join("rex");
        write_bin(
            &bin,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{dir}/args'\ncat > '{dir}/stdin'\nprintf '%s' '{stdout}'\nexit {status}\n",
                dir = dir.display()
            ),
        );
        bin
    }

    #[tokio::test]
    async fn ask_runs_rex_do_with_the_question_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_rex(dir.path(), r#"{"answer":"alice"}"#, 0);
        let palette = RexPalette::new(bin);
        assert_eq!(
            palette.ask(&pick()).await,
            PaletteReply::Answer("alice".into())
        );
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        assert_eq!(args.lines().collect::<Vec<_>>(), ARGV);
        let stdin: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("stdin")).unwrap())
                .unwrap();
        assert_eq!(stdin["ask"]["kind"], "pick");
        assert_eq!(stdin["ask"]["items"][0]["id"], "alice");
        assert_eq!(stdin["timeout"], 300);
    }

    #[tokio::test]
    async fn a_failed_rex_is_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        // For example, the server has no chaoshub_ask action.
        let bin = fake_rex(dir.path(), "", 3);
        let palette = RexPalette::new(bin);
        assert!(matches!(
            palette.ask(&pick()).await,
            PaletteReply::Fallback(_)
        ));
        let missing = RexPalette::new(dir.path().join("no-rex"));
        assert!(matches!(
            missing.ask(&pick()).await,
            PaletteReply::Fallback(_)
        ));
    }

    #[tokio::test]
    async fn detect_needs_revision_17_the_action_and_an_app() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("rex");
        let write = |version: u64, action: &str, kind: &str| {
            write_bin(
                &bin,
                &format!(
                    "#!/bin/sh\ncase \"$1\" in\napi) printf '{{\"api_revision\":{version}}}' ;;\nactions) printf '{{\"actions\":[{{\"name\":\"{action}\"}}]}}' ;;\nclient) printf '{{\"clients\":[{{\"info\":{{\"kind\":\"{kind}\"}}}}]}}' ;;\nesac\n"
                ),
            );
        };
        let palette = RexPalette::new(bin.clone());
        write(17, "chaoshub_ask", "app");
        assert!(palette.detect().await);
        write(16, "chaoshub_ask", "app");
        assert!(!palette.detect().await);
        write(17, "chaoshub_open_pr", "app");
        assert!(!palette.detect().await, "no chaoshub_ask action");
        write(17, "chaoshub_ask", "cli");
        assert!(!palette.detect().await);
        let missing = RexPalette::new(dir.path().join("no-rex"));
        assert!(!missing.detect().await);
    }
}
