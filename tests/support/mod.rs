//! Shared test utilities for the `sebas` integration tests.
//!
//! `TestDir` is the canonical scratch-directory primitive: each instance
//! gets a fresh, unique subdirectory under `target/tests/<crate>/<test>/`
//! and removes it on drop. Tests that need a stable path they can hand to
//! a child process should keep the `TestDir` alive for the duration of
//! the test (or call `keep()` if they need it to outlive the test).
//!
//! Why `target/tests/` instead of `/tmp` or `$HOME`:
//!
//! - **Hermetic.** Runs don't share `/tmp` with the rest of the host, so
//!   parallel CI agents and concurrent local runs can't collide.
//! - **Owned by cargo.** Lives under the workspace `target/`, so a plain
//!   `cargo clean` (which the user just ran) wipes every stale scratch
//!   dir along with the build artefacts. No `~/.local/state` pollution.
//! - **Predictable layout.** `target/tests/<crate>/<test_name>/<unique>/`
//!   means a failing test's leftover state is trivial to find.
//!
//! Layout is computed from `CARGO_MANIFEST_DIR`, which cargo sets for
//! every test binary at compile time.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// RAII scratch directory rooted at `target/tests/<crate>/<test>/<unique>`.
///
/// `Drop` removes the directory and everything under it. Call `keep()`
/// to leak the directory (useful when the test deliberately crashes the
/// daemon and you want the leftover state inspectable afterwards — the
/// next `cargo clean` will still tidy up).
pub struct TestDir {
    path: PathBuf,
    keep: bool,
}

impl TestDir {
    /// Create a fresh scratch dir for `test_name`. `sub` lets one test
    /// claim multiple disjoint dirs (e.g. one for state, one for config).
    pub fn new(test_name: &str, sub: &str) -> Self {
        Self::with_crate(test_name, sub, env!("CARGO_PKG_NAME"))
    }

    /// Same as `new` but with the crate name spelled explicitly. Use
    /// this from `gateway/tests/support/mod.rs` (a different crate's
    /// `CARGO_PKG_NAME`).
    pub fn with_crate(test_name: &str, sub: &str, crate_name: &str) -> Self {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| {
            panic!("CARGO_MANIFEST_DIR unset — TestDir must be called from a cargo test binary")
        });
        let manifest = PathBuf::from(manifest_dir);
        // Workspace root is the parent of every member crate's manifest dir
        // (sebas's workspace layout: <root>/{router,feishu,...}/Cargo.toml).
        // The fallback `manifest.clone()` covers single-crate checkouts
        // where the test crate IS the workspace root.
        let workspace_root = manifest
            .parent()
            .filter(|p| p.join("Cargo.toml").exists())
            .map(|p| p.to_path_buf())
            .unwrap_or(manifest);
        let stamp = unique_stamp();
        let path = workspace_root
            .join("target")
            .join("tests")
            .join(crate_name)
            .join(test_name)
            .join(format!("{stamp}-{sub}"));
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("create scratch dir {}: {e}", path.display()));
        Self { path, keep: false }
    }

    /// Path to the scratch directory. Created; safe to write into.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Disable auto-cleanup on drop. Use when you want the test's
    /// leftovers to survive a crash for postmortem inspection.
    pub fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        // Best-effort: a parallel test might be holding a handle, or
        // permission might already be revoked (unlikely on target/, but
        // be defensive). Ignore failures — `cargo clean` is the
        // hammer-of-last-resort.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Combination of nanos-since-epoch + a process-local counter, so two
/// `TestDir::new` calls inside the same `#[tokio::test]` don't race on
/// the timestamp and end up sharing a path.
fn unique_stamp() -> u128 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    (t << 16) | (n & 0xFFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_unique_paths_and_cleans_up() {
        let a = TestDir::new("self", "alpha");
        let b = TestDir::new("self", "beta");
        assert_ne!(
            a.path(),
            b.path(),
            "unique stamps must produce distinct paths"
        );
        assert!(a.path().exists());
        assert!(b.path().exists());
        let pa = a.path().to_path_buf();
        let pb = b.path().to_path_buf();
        drop(a);
        drop(b);
        assert!(!pa.exists(), "alpha dir must be removed on drop");
        assert!(!pb.exists(), "beta dir must be removed on drop");
    }

    #[test]
    fn keep_survives_drop() {
        let mut d = TestDir::new("self", "keep");
        d.keep();
        let p = d.path().to_path_buf();
        drop(d);
        assert!(p.exists(), "keep() must prevent cleanup");
        // Tidy up so the test itself is hermetic.
        std::fs::remove_dir_all(&p).unwrap();
    }

    #[test]
    fn path_lives_under_target_tests() {
        let d = TestDir::new("self", "layout");
        let path = d.path();
        assert!(
            path.components().any(|c| c.as_os_str() == "tests"),
            "TestDir path must include a `tests/` segment under target/: {}",
            path.display()
        );
        assert!(
            path.to_string_lossy().contains("target"),
            "TestDir path must be under `target/`: {}",
            path.display()
        );
    }
}
