//! `PUBLISHER_REV`: a fingerprint of the code a headless MQTT publisher runs (everything under
//! `src/` except the TUI and self-update, plus `Cargo.lock`). Publishers report it, so the
//! dashboard can tell a devbox that needs updating from one that only missed a screen change.

use std::path::{Path, PathBuf};

/// Not the publisher's: drawing the dashboard and updating it in place.
const EXCLUDED: [&str; 3] = ["src/tui.rs", "src/tui", "src/self_update.rs"];

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path.to_string_lossy().replace('\\', "/");
        if EXCLUDED.iter().any(|e| relative == *e) {
            continue;
        }
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// FNV-1a, 64-bit: stable across compilers, unlike std's hasher.
fn fnv(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100_0000_01b3);
    }
}

/// `Cargo.lock` without this package's own version line, which every release bumps whether or
/// not the publishing code changed.
fn without_own_version(lock: &str) -> String {
    let mut out = String::new();
    let mut ours = false;
    for line in lock.lines() {
        if line.starts_with("name = ") {
            ours = line == "name = \"splitrail-dashboard\"";
        } else if ours && line.starts_with("version = ") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn main() {
    let mut paths = Vec::new();
    files(Path::new("src"), &mut paths);
    paths.push(PathBuf::from("Cargo.lock"));
    paths.sort();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for path in &paths {
        fnv(&mut hash, path.to_string_lossy().replace('\\', "/").as_bytes());
        let mut bytes = std::fs::read(path).unwrap_or_default();
        if path == Path::new("Cargo.lock") {
            bytes = without_own_version(&String::from_utf8_lossy(&bytes)).into_bytes();
        }
        fnv(&mut hash, &bytes);
    }
    println!("cargo:rustc-env=PUBLISHER_REV={hash:016x}");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.lock");
}
