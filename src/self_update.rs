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
        command.current_dir(source_dir()).stdin(std::process::Stdio::null());
        if let Some(log) = log_path().and_then(|p| std::fs::File::create(p).ok())
            && let Ok(err) = log.try_clone()
        {
            command.stdout(log).stderr(err);
        }
        match command.status() {
            Ok(status) if status.success() => {
                set(State::Ready(version));
                RESTART.store(true, Ordering::Relaxed);
            }
            Ok(_) => set(State::Failed(version)),
            Err(e) => {
                crate::utils::log_error(format!("Update: could not run cargo: {e}"));
                set(State::Failed(version));
            }
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
            command
                .args([&target.host, &broker, &user])
                .current_dir(source_dir())
                .stdin(std::process::Stdio::null());
            if let Some(out) = log.as_ref().and_then(|l| l.try_clone().ok())
                && let Ok(err) = out.try_clone()
            {
                command.stdout(out).stderr(err);
            }
            let ok = command.status().is_ok_and(|s| s.success());
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

/// Replace this process with the newly built binary, same arguments. Only returns on failure.
pub fn exec_new_binary() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    // The path this process was started from, which the build just replaced.
    let binary = std::env::current_exe().unwrap_or_else(|_| {
        let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        source_dir().join("target").join(profile).join(BINARY)
    });
    std::process::Command::new(binary).args(std::env::args_os().skip(1)).exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(is_newer("2.0.0-dash-0.51", "2.0.0-dash-0.50"));
        assert!(is_newer("2.0.0-dash-0.100", "2.0.0-dash-0.99"));
        assert!(!is_newer("2.0.0-dash-0.50", "2.0.0-dash-0.50"));
        assert!(!is_newer("2.0.0-dash-0.49", "2.0.0-dash-0.50"));
    }

    #[test]
    fn reads_the_package_version() {
        let manifest = "[package]\nname = \"x\"\nversion = \"2.0.0-dash-0.52\"\n\n[dependencies]\nfoo = { version = \"1\" }\n";
        assert_eq!(manifest_version(manifest).as_deref(), Some("2.0.0-dash-0.52"));
        assert_eq!(manifest_version("not toml ["), None);
    }
}
