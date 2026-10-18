//! Path discovery for the platforms this Mac is not (BZL-12).
//!
//! The analyzers emit every platform's glob patterns unconditionally and let the filesystem
//! decide which exist, so a fake home laid out the Linux or Windows way can be discovered from
//! here. That keeps the cross-platform work testable without an Ubuntu or Windows box.
//!
//! One test function, because these manipulate process-wide environment variables.

use splitrail_dashboard::analyzer::Analyzer;
use splitrail_dashboard::analyzers::cline::ClineAnalyzer;
use splitrail_dashboard::analyzers::kilo_code::KiloCodeAnalyzer;

/// Restores `HOME` and `APPDATA` when it drops, whatever the test does.
struct EnvGuard {
    home: Option<std::ffi::OsString>,
    appdata: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn new() -> Self {
        Self {
            home: std::env::var_os("HOME"),
            appdata: std::env::var_os("APPDATA"),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match &self.appdata {
                Some(v) => std::env::set_var("APPDATA", v),
                None => std::env::remove_var("APPDATA"),
            }
        }
    }
}

/// Writes the task file and returns the task *directory*, which is what discovery yields.
fn write_task(root: &std::path::Path, relative: &str) -> std::path::PathBuf {
    let file = root.join(relative);
    let task_dir = file.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&task_dir).unwrap();
    // Enough shape that discovery counts it; parsing is covered elsewhere.
    std::fs::write(&file, "[]").unwrap();
    task_dir
}

#[test]
fn analyzers_discover_linux_and_windows_layouts_from_macos() {
    let _guard = EnvGuard::new();
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    // A Linux install: XDG-style ~/.config, which is where VSCode puts globalStorage there.
    let linux = write_task(
        home,
        ".config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/t-linux/ui_messages.json",
    );
    // A macOS install, for contrast: the same data under Application Support.
    let macos = write_task(
        home,
        "Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/t-mac/ui_messages.json",
    );

    unsafe {
        std::env::set_var("HOME", home);
        std::env::remove_var("APPDATA");
    }

    let found: Vec<std::path::PathBuf> = ClineAnalyzer::new()
        .discover_data_sources()
        .expect("discovery")
        .into_iter()
        .map(|s| s.path)
        .collect();

    assert!(
        found.iter().any(|p| p == &linux),
        "the Linux ~/.config layout must be discovered: {found:?}"
    );
    assert!(
        found.iter().any(|p| p == &macos),
        "the macOS layout must still be discovered: {found:?}"
    );

    // A Windows install is keyed off APPDATA rather than the home directory. The patterns are
    // emitted on every platform, so the pattern list is what we can assert from here.
    unsafe {
        std::env::set_var("APPDATA", r"C:\Users\test\AppData\Roaming");
    }
    let patterns = ClineAnalyzer::new().get_data_glob_patterns();
    assert!(
        patterns.iter().any(
            |p| p.starts_with(r"C:\Users\test\AppData\Roaming\") && p.contains("globalStorage")
        ),
        "an APPDATA pattern must be offered for Windows: {patterns:?}"
    );

    // Kilo Code covers the VSCode server layout too, which is what a headless Ubuntu box uses.
    let kilo = KiloCodeAnalyzer::new().get_data_glob_patterns();
    assert!(
        kilo.iter().any(|p| p.contains("/.config/")),
        "Kilo Code must offer the Linux desktop layout: {kilo:?}"
    );
    assert!(
        kilo.iter().any(|p| p.contains("vscode-server")),
        "Kilo Code must offer the VSCode server layout a remote Ubuntu box uses: {kilo:?}"
    );
}
