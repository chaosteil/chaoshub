//! The composition root: the event loop, the reader thread, the terminal
//! guard, the panic hook, the transitions and the draw.
//!
//! Threads: the event loop runs on the main thread and is the only code
//! that touches the terminal. The reader thread reads terminal events. The
//! signal thread turns SIGTERM and SIGINT into `Msg::Signal`. The tokio
//! runtime runs the workers and the action tasks.

use std::io::{self, Stdout, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::app::App;
use crate::app::exec::Runtime;
use crate::app::keymap::Keymap;
use crate::app::layout::layout;
use crate::bus::{Msg, Progress, Size, TermState, TermStatus};
use crate::cli::{CacheAction, CacheCommand, Cli};
use crate::config::schema::parse_repo_name;
use crate::config::{self, Config, paths};
use crate::domain::repo::Repo;
use crate::launch::rex::RexLauncher;
use crate::osc;
use crate::store::handle::StoreHandle;
use crate::store::{Snapshot, Store};
use crate::theme::{Theme, ThemeKind};
use crate::ui;
use crate::ui::transition::Transitions;

/// The capacity of the message channel.
const CHANNEL_CAPACITY: usize = 256;
/// The longest time step that one transition frame advances.
const MAX_ELAPSED: Duration = Duration::from_millis(32);
/// The poll timeout of the reader thread.
const READER_POLL: Duration = Duration::from_millis(100);
/// The longest wait for the reader thread to confirm a pause.
const PAUSE_TIMEOUT: Duration = Duration::from_secs(1);
/// The interval of the OSC 9;4 keep-alive while a progress is active.
/// Ghostty resets a progress bar after about 15 s without an update.
const PROGRESS_KEEP_ALIVE: Duration = Duration::from_secs(1);

/// The crossterm terminal of the event loop.
type CrosstermTerminal = Terminal<CrosstermBackend<Stdout>>;

/// Runs the command of `cli`: `cache clear`, or the TUI until the user
/// quits.
pub(crate) fn run(cli: Cli, theme_kind: ThemeKind) -> anyhow::Result<()> {
    let started = Instant::now();
    let Cli {
        repo,
        config: config_path,
        no_prefetch,
        command,
    } = cli;
    if let Some(CacheCommand::Cache(CacheAction::Clear)) = command {
        return clear_cache();
    }
    let config_path = config_path
        .or_else(paths::config_file)
        .context("the config directory is unknown")?;
    let mut config = config::load(&config_path)?;
    if no_prefetch {
        config.prefetch.enabled = false;
    }
    let keymap = Keymap::new(&config.keys)?;
    let theme = Theme::new(theme_kind, config.theme.grunge, config.theme.textures);
    let transitions =
        Transitions::new(config.theme.animations, config.theme.animation_ms).with_theme(theme);
    let detected = term_status_detected();
    let term = TermWriter::new(
        config.terminal.program_status.unwrap_or(detected),
        config.terminal.progress.unwrap_or(detected),
    );
    let launcher = launcher(&config, RexLauncher::from_env())?;
    let settings = Settings {
        repos: session_repos(&config, &repo)?,
        launcher,
        config,
    };
    tracing::debug!(?theme_kind, ?settings, "settings");
    let (snapshot, _store) = open_store();

    let tokio = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);

    let guard = TerminalGuard::enter()?;
    install_panic_hook(thread::current().id(), term.clear_sequence());
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let area = terminal.size()?;
    let size = Size {
        width: area.width,
        height: area.height,
    };

    let reader = Reader::spawn(tx.clone())?;
    let signals = spawn_signals(tx)?;
    let mut event_loop = EventLoop {
        app: App::new(
            size,
            keymap,
            snapshot,
            settings.repos.clone(),
            chrono::Utc::now(),
        ),
        runtime: Runtime::new(tokio.handle().clone()),
        handle: tokio.handle().clone(),
        transitions,
        theme,
        term,
        settings,
        drawn: None,
        was_active: false,
        last_draw: None,
        started: Some(started),
    };
    let result = event_loop.run(&mut terminal, rx);
    if let Err(err) = event_loop.term.clear(&mut io::stdout()) {
        tracing::warn!(%err, "cannot clear the terminal status");
    }

    signals.close();
    reader.stop();
    drop(guard);
    tokio.shutdown_timeout(Duration::from_millis(500));
    result
}

/// Opens the cache and reads the startup snapshot. When the cache does
/// not open, the app runs on an empty in-memory cache.
fn open_store() -> (Snapshot, StoreHandle) {
    let span = tracing::info_span!("store_open").entered();
    let started = Instant::now();
    let opened = paths::cache_db()
        .context("the cache directory is unknown")
        .and_then(|path| Store::open(&path).map_err(anyhow::Error::from));
    let opened = opened.or_else(|err| {
        tracing::warn!(%err, "cannot open the cache, using an empty one");
        Store::open(Path::new(":memory:"))
    });
    let result = match opened {
        Ok(opened) => opened,
        Err(err) => {
            tracing::error!(%err, "cannot open an in-memory cache");
            let (tx, _rx) = mpsc::channel(1);
            (Snapshot::default(), StoreHandle::new(tx))
        }
    };
    tracing::info!(
        ms = started.elapsed().as_millis(),
        prs = result.0.prs.len(),
        "store loaded"
    );
    drop(span);
    result
}

/// The settings that `run` builds from the configuration and the command
/// line, for the parts of the app that start after the first frame.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "phase D builds the Session and the workers from it, and phase E gives the launcher to the runtime"
)]
struct Settings {
    /// The configuration, with `--no-prefetch` applied.
    config: Config,
    /// The repos of the session: the `[[repo]]` entries, then the
    /// `--repo` repos.
    repos: Vec<Repo>,
    /// The launcher of detached panes. `None` runs a terminal command in
    /// the foreground.
    launcher: Option<RexLauncher>,
}

/// The repos of the session: the `[[repo]]` entries of the
/// configuration, then each `--repo` that is not one of them. A `--repo`
/// repo has no local path.
fn session_repos(config: &Config, extra: &[String]) -> anyhow::Result<Vec<Repo>> {
    let mut repos = config.repos();
    for text in extra {
        let key = parse_repo_name(text)
            .with_context(|| format!("--repo `{text}` is not in the owner/name form"))?;
        if !repos.iter().any(|repo| repo.key == key) {
            repos.push(Repo {
                key,
                path: None,
                settings: None,
            });
        }
    }
    Ok(repos)
}

/// The launcher for `launcher = "auto" | "rex" | "inline"`. `auto`, the
/// default, and `rex` use the Rex pane of the environment (`env`), with
/// the split and the ratio of `[codeview]`. `inline` uses none.
fn launcher(config: &Config, env: Option<RexLauncher>) -> anyhow::Result<Option<RexLauncher>> {
    let rex = env.map(|rex| RexLauncher {
        split: config.codeview.split.clone(),
        ratio: config.codeview.ratio,
        ..rex
    });
    match config.launcher.as_deref().unwrap_or("auto") {
        "auto" => Ok(rex),
        "rex" => {
            if rex.is_none() {
                tracing::warn!("launcher = \"rex\", but REX_BLOCK is not set");
            }
            Ok(rex)
        }
        "inline" => Ok(None),
        other => anyhow::bail!(
            "invalid launcher `{other}` in the configuration: use auto, rex or inline"
        ),
    }
}

/// `chaoshub cache clear`: deletes the cache file and says what it did.
fn clear_cache() -> anyhow::Result<()> {
    let path = paths::cache_db().context("the cache directory is unknown")?;
    let deleted = remove_cache(&path)
        .with_context(|| format!("cannot delete the cache {}", path.display()))?;
    if deleted {
        println!("deleted {}", path.display());
    } else {
        println!("no cache at {}", path.display());
    }
    Ok(())
}

/// Deletes the file at `path`. False when there was no file.
fn remove_cache(path: &std::path::Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// The state of the event loop on the main thread.
struct EventLoop {
    app: App,
    runtime: Runtime,
    handle: Handle,
    transitions: Transitions,
    theme: Theme,
    term: TermWriter,
    #[allow(
        dead_code,
        reason = "phase D builds the Session and the workers from it, and phase E gives the launcher to the runtime"
    )]
    settings: Settings,
    /// The generation of the last draw. `None` before the first draw.
    drawn: Option<u64>,
    /// True when a transition ran after the last draw.
    was_active: bool,
    /// The time of the last draw.
    last_draw: Option<Instant>,
    /// The start of `run`, until the first frame logs the startup time.
    started: Option<Instant>,
}

impl EventLoop {
    /// Runs until `Cmd::Quit` or until every sender is gone. `rx` drops
    /// on return, so a reader thread that waits on a full channel stops.
    fn run(
        &mut self,
        terminal: &mut CrosstermTerminal,
        mut rx: mpsc::Receiver<Msg>,
    ) -> anyhow::Result<()> {
        while !self.runtime.quitting() {
            let active = self.transitions.active();
            if active || self.drawn != Some(self.app.generation()) {
                let now = Instant::now();
                let elapsed = frame_elapsed(self.last_draw, now, self.was_active);
                self.drawn = Some(self.app.generation());
                self.last_draw = Some(now);
                draw(
                    terminal,
                    &mut self.app,
                    &mut self.transitions,
                    &self.theme,
                    elapsed,
                )?;
                if let Some(started) = self.started.take() {
                    tracing::info!(ms = started.elapsed().as_millis(), "startup: first frame");
                }
            }
            let active = self.transitions.active();
            if !active && (self.was_active || self.app.transition_leftovers()) {
                self.update(Msg::TransitionsDone);
            }
            self.was_active = active;
            let status = &self.app.frame().term;
            if let Err(err) = self.term.write(&mut io::stdout(), status, Instant::now()) {
                tracing::warn!(%err, "cannot write the terminal status");
            }

            let wait = async { timeout(self.transitions.deadline(), rx.recv()).await };
            match self.handle.block_on(wait) {
                Ok(Some(msg)) => self.update(msg),
                Ok(None) => break,
                Err(_) => self.update(Msg::Tick),
            }
        }
        Ok(())
    }

    /// Runs `update` and executes the commands.
    fn update(&mut self, msg: Msg) {
        for cmd in self.app.update(msg) {
            self.runtime.exec(cmd);
        }
    }
}

/// The time step of a transition frame: clamped to `MAX_ELAPSED`, and zero
/// on the first frame after idle.
fn frame_elapsed(last_draw: Option<Instant>, now: Instant, was_active: bool) -> Duration {
    match last_draw {
        Some(last) if was_active => now.saturating_duration_since(last).min(MAX_ELAPSED),
        _ => Duration::ZERO,
    }
}

/// Draws one frame: the frame model, the transitions and the view.
fn draw<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    transitions: &mut Transitions,
    theme: &Theme,
    elapsed: Duration,
) -> Result<(), B::Error> {
    let started = Instant::now();
    transitions.apply(app.take_ui_events());
    let areas = layout(app.size());
    let model = app.frame();
    transitions.prepare(model, &areas);
    terminal.draw(|frame| {
        ui::view(frame, model, &areas, theme);
        transitions.process(frame, elapsed);
    })?;
    tracing::trace!(render_us = started.elapsed().as_micros(), "frame");
    Ok(())
}

/// Puts the terminal in raw mode on the alternate screen, and restores it
/// on drop.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Leaves raw mode and the alternate screen.
fn restore_terminal() {
    if let Err(err) = disable_raw_mode() {
        tracing::warn!(%err, "cannot leave raw mode");
    }
    if let Err(err) = execute!(io::stdout(), Show, LeaveAlternateScreen) {
        tracing::warn!(%err, "cannot leave the alternate screen");
    }
}

/// True inside Rex (`REX_BLOCK` set) and Ghostty (`TERM_PROGRAM=ghostty`),
/// the terminals that show OSC 7501 and OSC 9;4.
fn term_status_detected() -> bool {
    std::env::var_os("REX_BLOCK").is_some()
        || std::env::var("TERM_PROGRAM").is_ok_and(|term| term == "ghostty")
}

/// Writes the terminal status: OSC 7501 when the state or the message
/// changed, OSC 9;4 when the progress changed or once per second while a
/// progress is active.
#[derive(Debug)]
struct TermWriter {
    /// The `[terminal] program_status` switch.
    program_status: bool,
    /// The `[terminal] progress` switch.
    progress: bool,
    /// The state and the message of the last OSC 7501 write.
    last_status: Option<(TermState, Option<String>)>,
    /// The progress and the time of the last OSC 9;4 write.
    last_progress: Option<(Progress, Instant)>,
}

impl TermWriter {
    fn new(program_status: bool, progress: bool) -> Self {
        Self {
            program_status,
            progress,
            last_status: None,
            last_progress: None,
        }
    }

    /// Writes what changed or what is due. Runs on every loop pass.
    fn write(&mut self, out: &mut impl Write, status: &TermStatus, now: Instant) -> io::Result<()> {
        let mut bytes = Vec::new();
        if self.program_status
            && self
                .last_status
                .as_ref()
                .is_none_or(|(state, msg)| *state != status.state || *msg != status.msg)
        {
            bytes.extend(osc::program_status(status));
            self.last_status = Some((status.state, status.msg.clone()));
        }
        if self.progress
            && self.last_progress.is_none_or(|(progress, at)| {
                progress != status.progress
                    || (status.progress != Progress::Off
                        && now.saturating_duration_since(at) >= PROGRESS_KEEP_ALIVE)
            })
        {
            bytes.extend(osc::progress(status.progress));
            self.last_progress = Some((status.progress, now));
        }
        if bytes.is_empty() {
            return Ok(());
        }
        out.write_all(&bytes)?;
        out.flush()
    }

    /// Writes `state=clear` and `9;4;0`, on quit and before the terminal
    /// is handed to a foreground process. The next `write` sends the
    /// status again.
    fn clear(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.last_status = None;
        self.last_progress = None;
        let bytes = self.clear_sequence();
        if bytes.is_empty() {
            return Ok(());
        }
        out.write_all(&bytes)?;
        out.flush()
    }

    /// The clear sequences of the switches that are on.
    fn clear_sequence(&self) -> Vec<u8> {
        match (self.program_status, self.progress) {
            (true, true) => osc::clear(),
            (true, false) => osc::program_status(&TermStatus {
                state: TermState::Clear,
                msg: None,
                progress: Progress::Off,
            }),
            (false, true) => osc::progress(Progress::Off),
            (false, false) => Vec::new(),
        }
    }
}

/// Clears the terminal status and restores the terminal before the panic
/// message when the event loop thread `main` panics. A panic on another
/// thread is only logged, because the event loop still owns the screen.
fn install_panic_hook(main: ThreadId, clear: Vec<u8>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(%info, "panic");
        if thread::current().id() == main {
            let mut out = io::stdout();
            if out.write_all(&clear).and_then(|()| out.flush()).is_err() {
                tracing::warn!("cannot clear the terminal status");
            }
            restore_terminal();
            previous(info);
        }
    }));
}

/// The reader thread: reads terminal events and sends them as messages.
/// The foreground terminal path pauses it, so that the child process owns
/// stdin.
struct Reader {
    pause: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    /// The reader sends one value when it is paused.
    #[allow(
        dead_code,
        reason = "the foreground RunTerminal path of phase E reads it"
    )]
    acks: std::sync::mpsc::Receiver<()>,
    thread: JoinHandle<()>,
}

impl Reader {
    fn spawn(tx: mpsc::Sender<Msg>) -> io::Result<Self> {
        let pause = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (ack_tx, acks) = std::sync::mpsc::channel();
        let thread = thread::Builder::new().name("reader".into()).spawn({
            let pause = Arc::clone(&pause);
            let stop = Arc::clone(&stop);
            move || read_loop(&tx, &pause, &stop, &ack_tx)
        })?;
        Ok(Self {
            pause,
            stop,
            acks,
            thread,
        })
    }

    /// Pauses the reader and waits until it confirms. False when it did
    /// not confirm in time.
    #[allow(
        dead_code,
        reason = "the foreground RunTerminal path of phase E calls it"
    )]
    fn pause(&self) -> bool {
        self.pause.store(true, Ordering::Release);
        self.thread.thread().unpark();
        self.acks.recv_timeout(PAUSE_TIMEOUT).is_ok()
    }

    /// Resumes a paused reader.
    #[allow(
        dead_code,
        reason = "the foreground RunTerminal path of phase E calls it"
    )]
    fn resume(&self) {
        self.pause.store(false, Ordering::Release);
        self.thread.thread().unpark();
    }

    /// Stops the reader and waits for its thread.
    fn stop(self) {
        self.stop.store(true, Ordering::Release);
        self.thread.thread().unpark();
        if self.thread.join().is_err() {
            tracing::error!("the reader thread panicked");
        }
    }
}

/// The loop of the reader thread. It checks the flags on every poll.
fn read_loop(
    tx: &mpsc::Sender<Msg>,
    pause: &AtomicBool,
    stop: &AtomicBool,
    ack: &std::sync::mpsc::Sender<()>,
) {
    let mut acked = false;
    while !stop.load(Ordering::Acquire) {
        if pause.load(Ordering::Acquire) {
            if !acked {
                acked = true;
                if ack.send(()).is_err() {
                    break;
                }
            }
            thread::park_timeout(READER_POLL);
            continue;
        }
        acked = false;
        match event::poll(READER_POLL) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(err) => {
                tracing::error!(%err, "cannot poll the terminal");
                break;
            }
        }
        let msg = match event::read() {
            Ok(Event::Key(key)) => Msg::Key(key),
            Ok(Event::Resize(width, height)) => Msg::Resize(Size { width, height }),
            Ok(_) => continue,
            Err(err) => {
                tracing::error!(%err, "cannot read the terminal");
                break;
            }
        };
        if tx.blocking_send(msg).is_err() {
            break;
        }
    }
}

/// Starts the signal thread: SIGTERM and SIGINT become `Msg::Signal`.
/// Closing the returned handle stops the thread.
fn spawn_signals(tx: mpsc::Sender<Msg>) -> io::Result<signal_hook::iterator::Handle> {
    let mut signals = Signals::new([SIGTERM, SIGINT])?;
    let handle = signals.handle();
    thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            for _ in signals.forever() {
                if tx.blocking_send(Msg::Signal).is_err() {
                    break;
                }
            }
        })?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::backend::TestBackend;

    use super::*;
    use crate::domain::keys::RepoKey;

    fn repo(owner: &str, name: &str, path: Option<&str>) -> Repo {
        Repo {
            key: RepoKey {
                owner: owner.into(),
                name: name.into(),
            },
            path: path.map(PathBuf::from),
            settings: None,
        }
    }

    #[test]
    fn session_repos_add_the_command_line_repos_once() {
        let config = Config::parse(
            "[[repo]]\nname = \"chaosteil/chaoshub\"\npath = \"/code/chaoshub\"\n",
            None,
        )
        .unwrap();
        let extra = [
            "pierredotco/deploy".to_owned(),
            "chaosteil/chaoshub".to_owned(),
        ];
        assert_eq!(
            session_repos(&config, &extra).unwrap(),
            [
                repo("chaosteil", "chaoshub", Some("/code/chaoshub")),
                repo("pierredotco", "deploy", None),
            ]
        );
        assert!(session_repos(&config, &["chaoshub".to_owned()]).is_err());
    }

    fn rex() -> RexLauncher {
        RexLauncher {
            bin: PathBuf::from("rex"),
            block: "block:01".into(),
            session: Some("session:01".into()),
            split: "right".into(),
            ratio: None,
        }
    }

    #[test]
    fn launcher_follows_the_setting() {
        let with = |text: &str| Config::parse(text, None).unwrap();
        let auto = with("[codeview]\nsplit = \"below\"\nratio = 70\n");
        assert_eq!(
            launcher(&auto, Some(rex())).unwrap(),
            Some(RexLauncher {
                split: "below".into(),
                ratio: Some(70),
                ..rex()
            })
        );
        assert_eq!(launcher(&auto, None).unwrap(), None);
        let forced = with("launcher = \"rex\"\n");
        assert_eq!(launcher(&forced, Some(rex())).unwrap(), Some(rex()));
        assert_eq!(launcher(&forced, None).unwrap(), None);
        let explicit_auto = with("launcher = \"auto\"\n");
        assert_eq!(launcher(&explicit_auto, Some(rex())).unwrap(), Some(rex()));
        let inline = with("launcher = \"inline\"\n");
        assert_eq!(launcher(&inline, Some(rex())).unwrap(), None);
        assert!(launcher(&with("launcher = \"tmux\"\n"), Some(rex())).is_err());
    }

    #[test]
    fn remove_cache_deletes_the_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chaoshub.db");
        std::fs::write(&path, "cache").unwrap();
        assert!(remove_cache(&path).unwrap());
        assert!(!path.exists());
        assert!(!remove_cache(&path).unwrap());
    }

    #[test]
    fn frame_elapsed_is_clamped_and_zero_after_idle() {
        let last = Instant::now();
        let later = last + Duration::from_millis(10);
        let much_later = last + Duration::from_millis(500);
        assert_eq!(frame_elapsed(None, later, true), Duration::ZERO);
        assert_eq!(frame_elapsed(Some(last), much_later, false), Duration::ZERO);
        assert_eq!(
            frame_elapsed(Some(last), later, true),
            Duration::from_millis(10)
        );
        assert_eq!(frame_elapsed(Some(last), much_later, true), MAX_ELAPSED);
    }

    fn working(progress: Progress) -> TermStatus {
        TermStatus {
            state: TermState::Working,
            msg: Some("syncing 2 repos".into()),
            progress,
        }
    }

    fn idle() -> TermStatus {
        TermStatus {
            state: TermState::Idle,
            msg: None,
            progress: Progress::Off,
        }
    }

    /// The bytes of one `write` call.
    fn written(writer: &mut TermWriter, status: &TermStatus, now: Instant) -> Vec<u8> {
        let mut out = Vec::new();
        writer.write(&mut out, status, now).unwrap();
        out
    }

    #[test]
    fn term_writer_writes_a_status_once() {
        let mut writer = TermWriter::new(true, true);
        let t0 = Instant::now();
        let mut first = osc::program_status(&idle());
        first.extend(osc::progress(Progress::Off));
        assert_eq!(written(&mut writer, &idle(), t0), first);
        assert_eq!(written(&mut writer, &idle(), t0), b"");
        assert_eq!(
            written(&mut writer, &idle(), t0 + Duration::from_secs(5)),
            b"",
            "no keep-alive without an active progress"
        );
    }

    #[test]
    fn term_writer_writes_only_what_changed() {
        let mut writer = TermWriter::new(true, true);
        let t0 = Instant::now();
        written(&mut writer, &idle(), t0);
        let busy = working(Progress::Off);
        assert_eq!(written(&mut writer, &busy, t0), osc::program_status(&busy));
        let busy = working(Progress::Indeterminate);
        assert_eq!(
            written(&mut writer, &busy, t0),
            osc::progress(Progress::Indeterminate)
        );
        let other_msg = TermStatus {
            msg: Some("syncing 3 repos".into()),
            ..working(Progress::Indeterminate)
        };
        assert_eq!(
            written(&mut writer, &other_msg, t0),
            osc::program_status(&other_msg)
        );
    }

    #[test]
    fn term_writer_keeps_an_active_progress_alive() {
        let mut writer = TermWriter::new(true, true);
        let t0 = Instant::now();
        let busy = working(Progress::Percent(40));
        written(&mut writer, &busy, t0);
        assert_eq!(
            written(&mut writer, &busy, t0 + Duration::from_millis(500)),
            b""
        );
        assert_eq!(
            written(&mut writer, &busy, t0 + Duration::from_secs(1)),
            osc::progress(Progress::Percent(40))
        );
        assert_eq!(
            written(&mut writer, &busy, t0 + Duration::from_millis(1500)),
            b""
        );
        assert_eq!(
            written(&mut writer, &busy, t0 + Duration::from_secs(2)),
            osc::progress(Progress::Percent(40))
        );
    }

    #[test]
    fn term_writer_clear_resets_the_dedupe() {
        let mut writer = TermWriter::new(true, true);
        let t0 = Instant::now();
        let first = written(&mut writer, &idle(), t0);
        let mut out = Vec::new();
        writer.clear(&mut out).unwrap();
        assert_eq!(out, osc::clear());
        assert_eq!(written(&mut writer, &idle(), t0), first);
    }

    #[test]
    fn term_writer_follows_the_switches() {
        let t0 = Instant::now();
        let busy = working(Progress::Indeterminate);
        let mut off = TermWriter::new(false, false);
        assert_eq!(written(&mut off, &busy, t0), b"");
        let mut out = Vec::new();
        off.clear(&mut out).unwrap();
        assert_eq!(out, b"");

        let mut status_only = TermWriter::new(true, false);
        assert_eq!(
            written(&mut status_only, &busy, t0),
            osc::program_status(&busy)
        );
        let mut progress_only = TermWriter::new(false, true);
        assert_eq!(
            written(&mut progress_only, &busy, t0),
            osc::progress(Progress::Indeterminate)
        );
        let mut out = Vec::new();
        progress_only.clear(&mut out).unwrap();
        assert_eq!(out, osc::progress(Progress::Off));
    }

    /// The app at a size with a snapshot, 5 minutes after the cache was
    /// written.
    fn seeded_app(width: u16, height: u16, snapshot: Snapshot) -> App {
        App::new(
            Size { width, height },
            Keymap::default(),
            snapshot,
            Vec::new(),
            crate::domain::test_time() + chrono::TimeDelta::minutes(5),
        )
    }

    /// Draws the app once without transitions and returns the screen.
    fn screen(app: &mut App) -> String {
        let size = app.size();
        let mut terminal = Terminal::new(TestBackend::new(size.width, size.height)).unwrap();
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        let mut transitions = Transitions::new(false, 160).with_theme(theme);
        draw(&mut terminal, app, &mut transitions, &theme, Duration::ZERO).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn mine_list_100() {
        let mut app = seeded_app(100, 14, crate::app::testdata::mine_snapshot());
        insta::assert_snapshot!(screen(&mut app));
    }

    #[test]
    fn mine_list_160() {
        let mut app = seeded_app(160, 14, crate::app::testdata::mine_snapshot());
        insta::assert_snapshot!(screen(&mut app));
    }

    /// The seeded app with the team fixtures, on the team tab.
    fn team_app(width: u16, height: u16) -> App {
        let mut app = seeded_app(width, height, crate::app::testdata::team_snapshot());
        let two = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('2'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.update(Msg::Key(two));
        app
    }

    #[test]
    fn team_list_100() {
        insta::assert_snapshot!(screen(&mut team_app(100, 12)));
    }

    #[test]
    fn team_list_160() {
        insta::assert_snapshot!(screen(&mut team_app(160, 12)));
    }

    fn press(app: &mut App, c: char) {
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        );
        app.update(Msg::Key(key));
    }

    #[test]
    fn author_picker_100() {
        let mut app = team_app(100, 16);
        press(&mut app, 'a');
        press(&mut app, 'j');
        insta::assert_snapshot!(screen(&mut app));
    }

    #[test]
    fn tab_slide_with_two_tabs() {
        let mut app = seeded_app(100, 12, crate::app::testdata::team_snapshot());
        let mut terminal = Terminal::new(TestBackend::new(100, 12)).unwrap();
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        let mut transitions = Transitions::new(true, 160).with_theme(theme);
        let mut frame = |app: &mut App, transitions: &mut Transitions, ms: u64| {
            draw(
                &mut terminal,
                app,
                transitions,
                &theme,
                Duration::from_millis(ms),
            )
            .unwrap();
            terminal.backend().to_string()
        };
        frame(&mut app, &mut transitions, 0);
        let two = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('2'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.update(Msg::Key(two));
        let start = frame(&mut app, &mut transitions, 0);
        assert!(transitions.active());
        let middle = frame(&mut app, &mut transitions, 80);
        let end = frame(&mut app, &mut transitions, 80);
        assert!(!transitions.active());
        insta::assert_snapshot!("app_tab_slide_0ms", start);
        insta::assert_snapshot!("app_tab_slide_80ms", middle);
        insta::assert_snapshot!("app_tab_slide_160ms", end);
        assert!(app.frame().prev_list.is_some());
        app.update(Msg::TransitionsDone);
        assert_eq!(app.frame().prev_list, None);
    }

    #[test]
    fn empty_frame() {
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        let size = Size {
            width: 40,
            height: 6,
        };
        let mut app = App::new(
            size,
            Keymap::default(),
            Snapshot::default(),
            Vec::new(),
            chrono::DateTime::UNIX_EPOCH,
        );
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        let mut transitions = Transitions::new(true, 160).with_theme(theme);
        draw(
            &mut terminal,
            &mut app,
            &mut transitions,
            &theme,
            Duration::ZERO,
        )
        .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
