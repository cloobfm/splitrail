//! Update in place: notice when the source tree this binary was built from has moved on to a
//! newer version, rebuild it on request, and restart into the new binary.
//!
//! The dashboard runs from its own checkout (`cargo run`), so "a new version" is the
//! `Cargo.toml` beside the code it was built from carrying a higher version than the running one.
//! `U` in the TUI builds the same profile (debug or release) with `cargo build`; once that
//! succeeds the TUI quits, the terminal is restored, and the process replaces itself with the new
//! binary, same arguments, same terminal (and same tmux pane).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The version this binary is.
pub const RUNNING: &str = env!("CARGO_PKG_VERSION");
const CHECK_INTERVAL: Duration = Duration::from_secs(60);
const BINARY: &str = "splitrail-dashboard";

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Current,
    /// A newer version in the source tree, not built yet.
    Available(String),
    Building(String),
    /// Built; restarting as soon as the TUI lets go.
    Ready(String),
    Failed(String),
}

static STATE: Mutex<State> = Mutex::new(State::Current);
static RESTART: AtomicBool = AtomicBool::new(false);

pub fn state() -> State {
    STATE.lock().map(|s| s.clone()).unwrap_or(State::Current)
}

fn set(state: State) {
    if let Ok(mut s) = STATE.lock() {
        *s = state;
    }
}

/// Whether the TUI should quit to restart into a freshly built binary.
pub fn restart_pending() -> bool {
    RESTART.load(Ordering::Relaxed)
}

fn source_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The `[package]` version in a `Cargo.toml`.
pub fn manifest_version(manifest: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(manifest).ok()?;
    Some(value.get("package")?.get("version")?.as_str()?.to_string())
}

/// `2.0.0-dash-0.51` as comparable numbers: every run of digits in order.
fn version_key(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

pub fn is_newer(candidate: &str, running: &str) -> bool {
    version_key(candidate) > version_key(running)
}

fn check() {
    let Ok(manifest) = std::fs::read_to_string(source_dir().join("Cargo.toml")) else {
        return; // Installed elsewhere, without its source: nothing to update from.
    };
    let Some(version) = manifest_version(&manifest) else {
        return;
    };
    let mut state = match STATE.lock() {
        Ok(state) => state,
        Err(_) => return,
    };
    // A build in flight or finished owns the state; a failure is retried once the source moves on.
    match &*state {
        State::Building(_) | State::Ready(_) => {}
        State::Failed(seen) if *seen == version => {}
        _ if is_newer(&version, RUNNING) => *state = State::Available(version),
        _ => *state = State::Current,
    }
}

/// Look for a newer version every minute while the dashboard runs.
pub async fn watch() {
    loop {
        tokio::task::spawn_blocking(check).await.ok();
        tokio::time::sleep(CHECK_INTERVAL).await;
    }
}

/// A build's progress: when it started and cargo's `done/total` crate count, by what is being
/// updated (`LOCAL`, or a publisher's slug).
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub started: std::time::Instant,
    pub done: u32,
    pub total: u32,
    /// How long the same update took last time, in seconds.
    pub expected: Option<u64>,
}

fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

impl Progress {
    /// `63% · 1:42` while many crates remain; otherwise the timer against last time's duration
    /// (`0:25 of ~0:55`). An incremental build spends nearly all its time on the last crate or
    /// two, where a percentage would sit at 99%.
    pub fn label(&self) -> String {
        let timer = clock(self.started.elapsed().as_secs());
        if self.total > 0 && self.total - self.done.min(self.total) > 3 {
            format!("{}% · {timer}", self.done * 100 / self.total)
        } else if let Some(expected) = self.expected {
            format!("{timer} of ~{}", clock(expected))
        } else {
            timer
        }
    }
}

fn durations_path() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(".config/splitrail/update_durations.json"))
}

/// Seconds each kind of update took last time, by key.
fn durations() -> serde_json::Map<String, serde_json::Value> {
    durations_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The key a duration is remembered under: the profile for this dashboard, the machine for a
/// publisher (the first one in a round also builds the Linux binary).
fn duration_key(key: &str) -> String {
    if key == LOCAL {
        format!("local-{}", if cfg!(debug_assertions) { "debug" } else { "release" })
    } else {
        format!("publisher-{key}")
    }
}

fn remember_duration(key: &str, secs: u64) {
    let mut all = durations();
    all.insert(duration_key(key), secs.into());
    if let Some(path) = durations_path() {
        let _ = std::fs::write(path, serde_json::Value::Object(all).to_string());
    }
}

pub const LOCAL: &str = "";

static PROGRESS: Mutex<Vec<(String, Progress)>> = Mutex::new(Vec::new());

pub fn progress(key: &str) -> Option<Progress> {
    PROGRESS.lock().ok()?.iter().find(|(k, _)| k == key).map(|(_, p)| *p)
}

fn set_progress(key: &str, change: impl FnOnce(&mut Progress)) {
    if let Ok(mut all) = PROGRESS.lock() {
        if let Some((_, p)) = all.iter_mut().find(|(k, _)| k == key) {
            change(p);
        } else {
            let expected = durations().get(&duration_key(key)).and_then(|v| v.as_u64());
            let mut p = Progress { started: std::time::Instant::now(), done: 0, total: 0, expected };
            change(&mut p);
            all.push((key.to_string(), p));
        }
    }
}

fn clear_progress(key: &str) {
    if let Ok(mut all) = PROGRESS.lock() {
        all.retain(|(k, _)| k != key);
    }
}

/// Cargo's `done/total` from one progress line (`Building [===>  ] 120/300: crate`).
pub fn parse_progress(line: &str) -> Option<(u32, u32)> {
    let rest = line.trim_start().strip_prefix("Building [")?;
    let counts = rest.split("] ").nth(1)?.split(':').next()?;
    let (done, total) = counts.split_once('/')?;
    Some((done.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// Run `command` with its output appended to `log`, tracking cargo's progress under `key`.
/// Cargo only draws progress for a terminal unless told to; those redraws are kept out of the log.
fn run_tracked(mut command: std::process::Command, log: Option<std::fs::File>, key: &str) -> bool {
    use std::io::{Read, Write};
    command
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "100")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    // Never the terminal: the TUI owns it.
    match log.as_ref().and_then(|l| l.try_clone().ok()) {
        Some(out) => command.stdout(out),
        None => command.stdout(std::process::Stdio::null()),
    };
    set_progress(key, |_| {});
    let Ok(mut child) = command.spawn() else {
        clear_progress(key);
        return false;
    };
    if let Some(mut stderr) = child.stderr.take() {
        let mut log = log;
        let mut buf = [0u8; 4096];
        let mut pending = Vec::new();
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            for &byte in &buf[..n] {
                if byte != b'\r' && byte != b'\n' {
                    pending.push(byte);
                    continue;
                }
                let line = String::from_utf8_lossy(&pending).into_owned();
                pending.clear();
                if let Some((done, total)) = parse_progress(&line) {
                    set_progress(key, |p| (p.done, p.total) = (done, total));
                } else if !line.trim().is_empty()
                    && let Some(log) = log.as_mut()
                {
                    let _ = writeln!(log, "{line}");
                }
            }
        }
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    if ok && let Some(progress) = progress(key) {
        remember_duration(key, progress.started.elapsed().as_secs());
    }
    clear_progress(key);
    ok
}

fn log_path() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(".config/splitrail/update.log"))
}

/// `U`: update this dashboard if the source has a newer version, otherwise every other machine
/// whose publisher is behind this build. Output goes to `~/.config/splitrail/update.log`.
pub fn start() {
    let version = match state() {
        State::Available(version) | State::Failed(version) => version,
        State::Current => {
            start_publishers();
            return;
        }
        _ => return,
    };
    set(State::Building(version.clone()));
    std::thread::spawn(move || {
        let mut command = std::process::Command::new("cargo");
        command.arg("build").arg("--bin").arg(BINARY);
        if !cfg!(debug_assertions) {
            command.arg("--release");
        }
        command.current_dir(source_dir());
        let log = log_path().and_then(|p| std::fs::File::create(p).ok());
        if run_tracked(command, log, LOCAL) {
            set(State::Ready(version));
            RESTART.store(true, Ordering::Relaxed);
        } else {
            set(State::Failed(version));
        }
    });
}

/// Publishers that are behind and online, not already being updated.
pub fn publishers_to_update() -> Vec<crate::rollup::Publisher> {
    crate::rollup::publishers()
        .into_iter()
        .filter(|p| p.online && p.behind() && p.update != crate::rollup::PublisherUpdate::Updating)
        .collect()
}

/// Reinstall each out-of-date publisher with `scripts/install_mqtt_publisher.sh`, one machine at
/// a time (the first builds the Linux binary; the rest reuse it). Each reports its new build over
/// MQTT once its service restarts, which clears it from the title.
fn start_publishers() {
    let targets = publishers_to_update();
    if targets.is_empty() {
        return;
    }
    let config = crate::config::Config::load().ok().flatten().unwrap_or_default().mqtt;
    let (Some(broker), Some(user)) = (config.host, config.username) else {
        crate::utils::log_error("Update: no MQTT broker configured to install publishers against");
        return;
    };
    for target in &targets {
        crate::rollup::set_publisher_update(&target.slug, crate::rollup::PublisherUpdate::Updating);
    }
    std::thread::spawn(move || {
        let log = log_path().and_then(|p| {
            std::fs::OpenOptions::new().create(true).append(true).open(p).ok()
        });
        for target in targets {
            let mut command = std::process::Command::new(source_dir().join("scripts/install_mqtt_publisher.sh"));
            command.args([&target.host, &broker, &user]).current_dir(source_dir());
            let ok = run_tracked(command, log.as_ref().and_then(|l| l.try_clone().ok()), &target.slug);
            if !ok {
                crate::utils::log_error(format!("Update: installing on {} failed; see ~/.config/splitrail/update.log", target.host));
            }
            crate::rollup::set_publisher_update(
                &target.slug,
                if ok { crate::rollup::PublisherUpdate::Idle } else { crate::rollup::PublisherUpdate::Failed },
            );
        }
    });
}

/// The path this process was started from, which a build will have replaced.
fn new_binary_path() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_else(|_| {
        let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        source_dir().join("target").join(profile).join(BINARY)
    })
}

/// Replace this process with the newly built binary, same arguments. Only returns on failure.
#[cfg(unix)]
pub fn exec_new_binary() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(new_binary_path())
        .args(std::env::args_os().skip(1))
        .exec()
}

/// Windows has no `exec`: the closest is to start the replacement and leave, so this spawns the
/// new binary and exits rather than returning into a process whose executable has been swapped.
#[cfg(windows)]
pub fn exec_new_binary() -> std::io::Error {
    match std::process::Command::new(new_binary_path())
        .args(std::env::args_os().skip(1))
        .spawn()
    {
        Ok(_) => std::process::exit(0),
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(is_newer("2.0.0-dash-0.51", "2.0.0-dash-0.50"));
        // Past 0.99 the number carries on: 0.100 is newer, not older.
        assert!(is_newer("2.0.0-dash-0.100", "2.0.0-dash-0.99"));
        assert!(is_newer("2.0.0-dash-0.100", "2.0.0-dash-0.99"));
        assert!(!is_newer("2.0.0-dash-0.50", "2.0.0-dash-0.50"));
        assert!(!is_newer("2.0.0-dash-0.49", "2.0.0-dash-0.50"));
    }

    #[test]
    fn reads_cargo_progress_lines() {
        assert_eq!(parse_progress("    Building [=====>      ] 120/300: serde, tokio"), Some((120, 300)));
        assert_eq!(parse_progress("    Building [             ] 0/2: itoa"), Some((0, 2)));
        assert_eq!(parse_progress("   Compiling itoa v1.0.15"), None);
        let progress = Progress { started: std::time::Instant::now(), done: 63, total: 100, expected: Some(55) };
        assert_eq!(progress.label(), "63% · 0:00");
        // Near the end of the crate list the percentage says nothing; last time's duration does.
        assert_eq!(Progress { done: 98, ..progress }.label(), "0:00 of ~0:55");
        assert_eq!(Progress { total: 0, expected: None, ..progress }.label(), "0:00");
    }

    #[test]
    fn reads_the_package_version() {
        let manifest = "[package]\nname = \"x\"\nversion = \"2.0.0-dash-0.52\"\n\n[dependencies]\nfoo = { version = \"1\" }\n";
        assert_eq!(manifest_version(manifest).as_deref(), Some("2.0.0-dash-0.52"));
        assert_eq!(manifest_version("not toml ["), None);
    }
}
