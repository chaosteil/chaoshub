//! The channel from the PR view processes to the list process.
//!
//! The list process binds a Unix datagram socket, `list-<pid>.sock`, in
//! a directory that only the user can open (mode 0700). It gives the
//! path to each PR view that it starts, in the `CHAOSHUB_LIST_SOCKET`
//! env. The PR view (`chaoshub pr`) sends one small JSON datagram for
//! each event, for example `{"v":1,"event":"approval_started",...}`.
//!
//! The channel goes one way only, and a send never waits:
//!
//! - When no list listens, the send fails, and the PR view ignores the
//!   failure.
//! - A message that does not decode is dropped.
//! - Each list process has its own socket, so a PR view talks only to the
//!   list that started it.
//! - At bind time, the list deletes the sockets of list processes that
//!   do not run now.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::bus::PrViewEvent;

/// The env that gives the socket of the list process to a PR view.
pub(crate) const ENV: &str = "CHAOSHUB_LIST_SOCKET";

/// The version of the message format. A message of another version is
/// dropped.
const VERSION: u32 = 1;

/// The largest message that the list reads. A longer datagram is cut,
/// and then it does not decode.
const MAX_MESSAGE: usize = 4096;

/// The longest wait of the listener thread for a datagram before it
/// looks at the stop flag again. The stop also wakes the thread with an
/// empty datagram, so this is only a fallback.
const STOP_POLL: Duration = Duration::from_secs(1);

/// The prefix and the suffix of the socket file names.
const PREFIX: &str = "list-";
const SUFFIX: &str = ".sock";

/// The message on the wire: the version and the event.
#[derive(Debug, Serialize, Deserialize)]
struct Wire {
    v: u32,
    #[serde(flatten)]
    event: PrViewEvent,
}

/// The JSON bytes of `event`.
pub(crate) fn encode(event: &PrViewEvent) -> Vec<u8> {
    let wire = Wire {
        v: VERSION,
        event: event.clone(),
    };
    serde_json::to_vec(&wire).unwrap_or_default()
}

/// The event of the JSON bytes `bytes`. `None` when the bytes are not a
/// message of this version.
pub(crate) fn decode(bytes: &[u8]) -> Option<PrViewEvent> {
    let wire: Wire = serde_json::from_slice(bytes).ok()?;
    (wire.v == VERSION).then_some(wire.event)
}

/// Sends `event` to the list socket at `path`. The send never blocks. A
/// failure, for example when no list listens, is only logged, because
/// the list then has nothing to update.
pub(crate) fn send(path: &Path, event: &PrViewEvent) {
    let result = UnixDatagram::unbound().and_then(|socket| {
        socket.set_nonblocking(true)?;
        socket.send_to(&encode(event), path)
    });
    match result {
        Ok(_) => tracing::debug!(?event, "told the list"),
        Err(err) => tracing::debug!(%err, ?event, path = %path.display(), "cannot tell the list"),
    }
}

/// The socket of the list process. Dropping it stops the listener thread
/// and deletes the socket file.
#[derive(Debug)]
pub(crate) struct Listener {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    /// Binds `list-<pid>.sock` in `dir` and starts the listener thread.
    /// The thread calls `on_event` for each event, until `on_event`
    /// returns false or the listener drops. The function makes `dir`
    /// with mode 0700, and gives the socket mode 0600. It first deletes
    /// the stale sockets in `dir`.
    pub(crate) fn bind(
        dir: &Path,
        mut on_event: impl FnMut(PrViewEvent) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        remove_stale(dir);
        let path = dir.join(format!("{PREFIX}{}{SUFFIX}", std::process::id()));
        let socket = UnixDatagram::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        socket.set_read_timeout(Some(STOP_POLL))?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = thread::Builder::new().name("ipc".into()).spawn(move || {
            let mut buf = vec![0; MAX_MESSAGE];
            while !flag.load(Ordering::Relaxed) {
                let len = match socket.recv(&mut buf) {
                    Ok(len) => len,
                    Err(err)
                        if matches!(
                            err.kind(),
                            io::ErrorKind::WouldBlock
                                | io::ErrorKind::TimedOut
                                | io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(err) => {
                        tracing::warn!(%err, "the list socket failed");
                        break;
                    }
                };
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let Some(event) = decode(&buf[..len]) else {
                    tracing::debug!(len, "dropped a message that does not decode");
                    continue;
                };
                if !on_event(event) {
                    break;
                }
            }
        });
        let thread = match thread {
            Ok(thread) => thread,
            Err(err) => {
                let _ = fs::remove_file(&path);
                return Err(err);
            }
        };
        Ok(Self {
            path,
            stop,
            thread: Some(thread),
        })
    }

    /// The path of the socket, for the env of the PR views.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // An empty datagram wakes the thread at once.
        if let Ok(socket) = UnixDatagram::unbound() {
            let _ = socket.set_nonblocking(true);
            let _ = socket.send_to(&[], &self.path);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Err(err) = fs::remove_file(&self.path) {
            tracing::debug!(%err, "cannot delete the list socket");
        }
    }
}

/// Deletes the list sockets in `dir` that no process listens on. A
/// connect to such a socket fails with "connection refused". The
/// sockets of the list processes that run stay.
fn remove_stale(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(PREFIX) && name.ends_with(SUFFIX)) {
            continue;
        }
        let path = entry.path();
        let refused = UnixDatagram::unbound()
            .and_then(|socket| socket.connect(&path))
            .is_err_and(|err| err.kind() == io::ErrorKind::ConnectionRefused);
        if refused {
            match fs::remove_file(&path) {
                Ok(()) => tracing::debug!(path = %path.display(), "deleted a stale list socket"),
                Err(err) => tracing::debug!(%err, "cannot delete a stale list socket"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::domain::keys::{PrKey, RepoKey};

    /// The longest wait of a test for an event.
    const WAIT: Duration = Duration::from_secs(5);

    fn pr(number: u64) -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: "chaosteil".into(),
                name: "chaoshub".into(),
            },
            number,
        }
    }

    /// A listener in `dir` that forwards each event to the receiver.
    fn listen(dir: &Path) -> (Listener, mpsc::Receiver<PrViewEvent>) {
        let (tx, rx) = mpsc::channel();
        let listener = Listener::bind(dir, move |event| tx.send(event).is_ok()).unwrap();
        (listener, rx)
    }

    #[test]
    fn events_encode_and_decode() {
        let events = [
            PrViewEvent::ApprovalStarted { pr: pr(7) },
            PrViewEvent::ApprovalDone {
                pr: pr(7),
                ok: true,
            },
            PrViewEvent::ApprovalDone {
                pr: pr(7),
                ok: false,
            },
            PrViewEvent::PrChanged { pr: pr(8) },
        ];
        for event in events {
            assert_eq!(decode(&encode(&event)), Some(event));
        }
    }

    #[test]
    fn the_wire_format_is_stable() {
        let json = String::from_utf8(encode(&PrViewEvent::ApprovalDone {
            pr: pr(7),
            ok: true,
        }))
        .unwrap();
        assert_eq!(
            json,
            r#"{"v":1,"event":"approval_done","pr":{"repo":{"owner":"chaosteil","name":"chaoshub"},"number":7},"ok":true}"#
        );
    }

    #[test]
    fn bad_messages_do_not_decode() {
        let cases: [&[u8]; 5] = [
            b"",
            b"not json",
            br#"{"v":2,"event":"pr_changed","pr":{"repo":{"owner":"a","name":"b"},"number":1}}"#,
            br#"{"v":1,"event":"merged","pr":{"repo":{"owner":"a","name":"b"},"number":1}}"#,
            br#"{"v":1,"event":"pr_changed"}"#,
        ];
        for bytes in cases {
            assert_eq!(decode(bytes), None, "{}", String::from_utf8_lossy(bytes));
        }
    }

    #[test]
    fn a_send_reaches_the_listener() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, rx) = listen(dir.path());
        send(listener.path(), &PrViewEvent::PrChanged { pr: pr(7) });
        assert_eq!(
            rx.recv_timeout(WAIT),
            Ok(PrViewEvent::PrChanged { pr: pr(7) })
        );
    }

    #[test]
    fn a_bad_message_is_dropped_and_the_listener_goes_on() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, rx) = listen(dir.path());
        let socket = UnixDatagram::unbound().unwrap();
        socket.send_to(b"garbage", listener.path()).unwrap();
        send(listener.path(), &PrViewEvent::ApprovalStarted { pr: pr(9) });
        assert_eq!(
            rx.recv_timeout(WAIT),
            Ok(PrViewEvent::ApprovalStarted { pr: pr(9) })
        );
    }

    #[test]
    fn a_send_without_a_listener_returns_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        // No file.
        send(
            &dir.path().join("list-1.sock"),
            &PrViewEvent::PrChanged { pr: pr(7) },
        );
        // A file that no process listens on.
        let path = dir.path().join("list-2.sock");
        drop(UnixDatagram::bind(&path).unwrap());
        send(&path, &PrViewEvent::PrChanged { pr: pr(7) });
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_full_socket_does_not_block_the_sender() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("list-3.sock");
        // A socket that nobody reads.
        let _socket = UnixDatagram::bind(&path).unwrap();
        let started = std::time::Instant::now();
        for _ in 0..10_000 {
            send(&path, &PrViewEvent::PrChanged { pr: pr(7) });
        }
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_socket_is_private_and_goes_away_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let ipc = dir.path().join("ipc");
        let (listener, _rx) = listen(&ipc);
        let path = listener.path().to_path_buf();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&ipc), 0o700);
        assert_eq!(mode(&path), 0o600);
        drop(listener);
        assert!(!path.exists());
    }

    #[test]
    fn bind_deletes_stale_sockets_and_keeps_live_ones() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("list-999999.sock");
        drop(UnixDatagram::bind(&stale).unwrap());
        let live = dir.path().join("list-999998.sock");
        let _live = UnixDatagram::bind(&live).unwrap();
        let other = dir.path().join("notes.txt");
        fs::write(&other, "x").unwrap();
        assert!(stale.exists());
        let (_listener, _rx) = listen(dir.path());
        assert!(!stale.exists(), "nobody listens on it");
        assert!(live.exists(), "another list listens on it");
        assert!(other.exists(), "not a list socket");
    }

    #[test]
    fn a_stale_socket_of_the_same_pid_does_not_stop_the_bind() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir
            .path()
            .join(format!("{PREFIX}{}{SUFFIX}", std::process::id()));
        drop(UnixDatagram::bind(&own).unwrap());
        let (listener, rx) = listen(dir.path());
        assert_eq!(listener.path(), own);
        send(&own, &PrViewEvent::PrChanged { pr: pr(1) });
        assert_eq!(
            rx.recv_timeout(WAIT),
            Ok(PrViewEvent::PrChanged { pr: pr(1) })
        );
    }
}
