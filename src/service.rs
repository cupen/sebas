//! `sebas service` — install/uninstall a systemd system unit for sebas.
//!
//! Pure functions live at the top (`render_unit`, `UnitInputs`). The
//! `run_install` / `run_uninstall` entry points handle argument validation,
//! FS effects, and the `systemctl` invocations. macOS is unsupported for the
//! systemd path; we exit 6 with a hint to hand-write a launchd plist.
//!
//! The service always runs as a non-root user: the unit carries
//! `User=<user>`/`Group=<user>` (chosen via `--user`), and `--user=root` is
//! rejected. The installer itself still requires root (writing
//! `/etc/systemd/system` + system-scope `systemctl`), but the daemon process
//! never runs as root.
//!
//! The unit runs the **watchdog** entrypoint (`<stable-binary> watchdog
//! --config <config>`), not the bare core: systemd supervises exactly one
//! process and the watchdog supervises core/gateway/webui, owns the control
//! socket + `services.json` desired-state, and performs self-upgrade. The
//! stable binary lives at `<data_dir>/bin/sebas` (seeded at install, replaced
//! in place by `sebas update`), so a machine reboot runs the latest version.
//!
//! Exit codes (from the brief):
//!   2 = `--install`/`--uninstall` action missing or both given
//!   3 = unit file already exists at `unit_path()` (and `!force`)
//!   4 = EUID/`--user` conflict (not root, or `--user` empty/root/nonexistent)
//!   5 = `--config` path missing or not absolute
//!   6 = unsupported platform (macOS)

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow};
use tracing::{info, warn};

/// Where the unit file will be written / is expected.
pub fn unit_path() -> PathBuf {
    PathBuf::from("/etc/systemd/system").join(UNIT_NAME)
}

#[derive(Debug)]
pub struct Args {
    /// `--install`: render + install the unit.
    pub install: bool,
    /// `--uninstall`: stop/disable/remove the unit.
    pub uninstall: bool,
    /// OS user the sebas service runs as (`User=`/`Group=`). Never root.
    pub user: String,
    /// After installing, also `systemctl enable --now` the unit.
    pub auto_start: bool,
    /// Overwrite an existing unit file.
    pub force: bool,
    /// Path to the sebas config.toml to bake into ExecStart. Must be absolute.
    pub config: String,
    /// Bake a specific `RUST_LOG` value into the unit instead of inheriting it.
    /// Empty means "inherit from the installing env, fall back to info".
    pub log_level: String,
}

/// Inputs to the pure renderer. Held by reference so tests can pass
/// `Path::new("…")` literals without allocation.
pub struct UnitInputs<'a> {
    /// Stable binary path (seeded at install). Rendered into `ExecStart`.
    pub binary_abs: &'a Path,
    pub config_abs: &'a Path,
    /// The OS user the service runs as. Rendered into both `User=` and
    /// `Group=`; the validators refuse `root`/empty/nonexistent.
    pub user: &'a str,
    pub log_level: &'a str,
}

const UNIT_NAME: &str = "sebas.service";
const DESCRIPTION: &str = "sebas — bridge Feishu ↔ Claude Code via ACP";

/// systemd escapes a single command word (`ExecStart` argument) so that
/// spaces/special chars stay one token. Mirrors systemd's quoting rule: wrap
/// the shell-escaped value in double quotes; `systemd` unquotes it.
fn systemd_escape(arg: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    out.push('"');
    for c in arg.chars() {
        // systemd's double-quoted tokens treat `"` and `\` specially.
        if c == '"' {
            // split the quote (shell-style continuation isn't needed here);
            // systemd does not support `\"` inside double quotes, so we drop
            // the risk by replacing with a single quote word-boundary via
            // reverting to shell escaping — a config path with `"` is not
            // expressible; reject it earlier. For now, escape defensively.
            write!(out, "\\\"").ok();
        } else if c == '\\' {
            write!(out, "\\\\").ok();
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// Render the full systemd unit file text. Pure: no FS, no system calls.
/// The unit tests in this module are the contract.
pub fn render_unit(inputs: UnitInputs<'_>) -> String {
    let mut s = String::new();

    // [Unit]
    s.push_str("[Unit]\n");
    s.push_str(&format!("Description={DESCRIPTION}\n"));
    s.push_str("After=network-online.target\n");
    s.push_str("Wants=network-online.target\n");
    s.push('\n');

    // [Service]
    s.push_str("[Service]\n");
    s.push_str("Type=simple\n");
    // Always drop privileges: run as the `--user` account, never root.
    s.push_str(&format!("User={}\n", inputs.user));
    s.push_str(&format!("Group={}\n", inputs.user));
    s.push_str(&format!("Environment=RUST_LOG={}\n", inputs.log_level));
    // The watchdog entrypoint: systemd supervises one process; watchdog
    // supervises core/gateway/webui + self-upgrade + control plane.
    s.push_str(&format!(
        "ExecStart={} watchdog --config {}\n",
        systemd_escape(&inputs.binary_abs.display().to_string()),
        systemd_escape(&inputs.config_abs.display().to_string()),
    ));
    s.push_str("Restart=on-failure\n");
    s.push_str("RestartSec=5\n");

    // Hardening. `ProtectSystem=full` (not `strict`): the watchdog must be
    // able to replace its own binary at `data_dir/bin` on self-upgrade, but
    // `/usr,/boot,/etc` stay read-only. `full` leaves `/home` and the data
    // dir writable while keeping system dirs read-only.
    s.push_str("NoNewPrivileges=true\n");
    s.push_str("ProtectSystem=full\n");
    s.push_str("ProtectHome=read-only\n");
    s.push_str("PrivateTmp=true\n");
    s.push('\n');

    // [Install]
    s.push_str("[Install]\n");
    s.push_str("WantedBy=multi-user.target\n");

    s
}

/// Return current effective UID. Wraps `libc::geteuid` so callers don't need
/// to plumb the unsafe import everywhere. (`nix` is not a dependency.)
fn current_euid() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: `geteuid` is async-signal-safe and has no preconditions.
        unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        // Non-Unix platforms (Windows) have no EUID concept. `run_*` exits 6
        // before this is ever consulted; return a non-zero placeholder so the
        // root check would fail loudly if that ever changes.
        1000
    }
}

/// Build a tagged `anyhow::Error` that carries the desired process exit
/// code. `main.rs` calls `exit_code_of(&err)` to decide what to exit with.
fn exit_err(code: i32, msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(ExitCode(code)).context(msg.into())
}

/// Look up the exit code carried by an `anyhow::Error`, if any. Walks the
/// `Error::context` chain.
pub fn exit_code_of(err: &anyhow::Error) -> Option<i32> {
    err.downcast_ref::<ExitCode>().map(|e| e.0)
}

#[derive(Debug)]
struct ExitCode(i32);
impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit {}", self.0)
    }
}
impl std::error::Error for ExitCode {}

/// Run the shared platform + privilege validation for both install and
/// uninstall: must be on systemd and running as root (writing `/etc/systemd`
/// and system-scope `systemctl`). Exits 6 (non-systemd) or 4 (not root).
///
/// The `--user` non-root check is separate; it only applies to `--install`.
fn validate_platform() -> anyhow::Result<()> {
    // systemd only exists on Linux; macOS has no systemd either. Exit 6.
    if cfg!(not(unix)) || cfg!(target_os = "macos") {
        return Err(exit_err(
            6,
            "systemd is not available on this platform; use `sebas run` directly",
        ));
    }

    // Installing/uninstalling a system unit writes /etc/systemd and runs
    // system scope `systemctl`; that requires root. The *service* itself
    // still drops to `--user`.
    if current_euid() != 0 {
        return Err(exit_err(
            4,
            "system unit install/uninstall requires root (run under sudo)",
        ));
    }

    Ok(())
}

/// Validate the install-time `--user`: never run the sebas daemon as root,
/// and the account must actually exist (the daemon drops to it at start via
/// `User=`/`Group=`, so a typo'd account would fail every boot).
fn validate_user(user: &str) -> anyhow::Result<()> {
    if user.is_empty() || user == "root" {
        return Err(exit_err(
            4,
            "--user must name a non-root account to run the service as",
        ));
    }
    if !user_exists(user) {
        return Err(exit_err(
            4,
            format!(
                "--user {user} does not exist on this system; create it first"
            ),
        ));
    }
    Ok(())
}

/// Does a POSIX account with this name exist? Uses `libc::getpwnam` (no child
/// process), matching the existing `libc::geteuid` dependency.
fn user_exists(user: &str) -> bool {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let name = match CString::new(user) {
            Ok(n) => n,
            Err(_) => return false, // embedded NUL: never a valid account
        };
        // SAFETY: `getpwnam` returns a pointer to static storage or NULL; we
        // only test for NULL and never deref the result beyond reading pw_dir.
        unsafe { !libc::getpwnam(name.as_ptr()).is_null() }
    }
    #[cfg(not(unix))]
    {
        let _ = user;
        false
    }
}

/// Home directory of a POSIX account (from `getpwnam().pw_dir`), used to
/// locate the default data dir when the config has no explicit `data_dir`.
/// Returns `None` if the account doesn't exist or has no home.
fn user_home(user: &str) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let name = CString::new(user).ok()?;
        // SAFETY: `getpwnam` returns a pointer to static storage we must not
        // free; we copy the pw_dir out immediately before any other call.
        let pw = unsafe { libc::getpwnam(name.as_ptr()) };
        if pw.is_null() {
            return None;
        }
        let pw = unsafe { &*pw };
        if pw.pw_dir.is_null() {
            return None;
        }
        let dir = unsafe { std::ffi::CStr::from_ptr(pw.pw_dir) }.to_string_lossy();
        if dir.is_empty() {
            None
        } else {
            Some(PathBuf::from(dir.to_string()))
        }
    }
    #[cfg(not(unix))]
    {
        let _ = user;
        None
    }
}

/// Validate an install-time `--config` path (absolute + exists). Exit 5.
fn validate_config(config: &str) -> anyhow::Result<PathBuf> {
    let config_path = PathBuf::from(config);
    if !config_path.is_absolute() {
        return Err(exit_err(
            5,
            format!("--config must be an absolute path, got: {config}"),
        ));
    }
    if !config_path.exists() {
        return Err(exit_err(
            5,
            format!(
                "config not found at {} (use --config)",
                config_path.display()
            ),
        ));
    }
    Ok(config_path)
}

/// Binary path: `current_exe()`, refusing non-absolute. Exit 4 defensively.
fn binary_abs() -> anyhow::Result<PathBuf> {
    let binary_abs = std::env::current_exe().context("resolve current_exe()")?;
    if !binary_abs.is_absolute() {
        return Err(exit_err(
            4,
            format!("current_exe() is not absolute: {}", binary_abs.display()),
        ));
    }
    Ok(binary_abs)
}

/// Run the `--install` flow. Errors map to sealed exit codes via `exit_code_of`.
pub async fn run_install(args: Args) -> anyhow::Result<()> {
    validate_platform()?;
    validate_user(&args.user)?;
    let config_path = validate_config(&args.config)?;
    let binary_abs = binary_abs()?;

    // Resolve the data dir from the config's [watchdog.storage].data_dir; when
    // unset, use the service user's home (NOT the installer's root home) so the
    // daemon can self-upgrade a data-dir-owned binary without root.
    let raw = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read config {}", config_path.display()))?;
    let cfg = crate::config::Config::parse(&raw).map_err(|e| anyhow!("parse config: {e}"))?;
    let data_dir = crate::upgrade::data_dir_for_user(
        cfg.watchdog.storage.data_dir.as_str(),
        user_home(&args.user),
    );

    // Seed the stable binary at <data_dir>/bin/sebas. `sebas update` replaces
    // this in place, so a machine reboot runs the newest installed version.
    let stable_bin = crate::upgrade::seed_stable_binary(&binary_abs, &data_dir)?;
    info!(
        stable = %stable_bin.display(),
        "seeded stable binary"
    );

    // Best-effort PATH convenience symlink. Not an error if it can't be
    // created (e.g. /usr/local/bin not writable / missing / on a restricted FS).
    #[cfg(unix)]
    {
        let link = PathBuf::from("/usr/local/bin/sebas");
        if let Some(parent) = link.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::os::unix::fs::symlink(&stable_bin, &link) {
            if link.exists() {
                let _ = std::fs::remove_file(&link);
                if std::os::unix::fs::symlink(&stable_bin, &link).is_err() {
                    warn!(path = %link.display(), "could not (re)create CLI symlink");
                }
            } else {
                warn!(path = %link.display(), "could not create CLI symlink: {e}");
            }
        }
    }

    let path = unit_path();
    if path.exists() && !args.force {
        return Err(exit_err(
            3,
            format!(
                "unit already exists at {}; use --force to overwrite",
                path.display()
            ),
        ));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create_dir_all({})", parent.display()))?;
    }

    // --log-level overrides the inherited RUST_LOG; empty falls back to the
    // installing env, then `info`.
    let log_level = if !args.log_level.is_empty() {
        args.log_level.clone()
    } else {
        std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())
    };
    let body = render_unit(UnitInputs {
        binary_abs: &stable_bin,
        config_abs: &config_path,
        user: &args.user,
        log_level: &log_level,
    });
    std::fs::write(&path, &body).with_context(|| format!("write unit file {}", path.display()))?;
    info!(path = %path.display(), "unit written");

    run_systemctl(&["daemon-reload"])?;

    if args.auto_start {
        run_systemctl(&["enable", "--now", UNIT_NAME])?;
    }

    // Idempotent re-install: if the unit is already active after reloading,
    // restart it so the new binary/config take effect deterministically.
    if is_unit_active() {
        run_systemctl(&["restart", UNIT_NAME])?;
        info!("restarted {UNIT_NAME}");
    }

    println!("Installed {} to {}", UNIT_NAME, path.display());
    println!("  binary: {}", stable_bin.display());
    if !args.auto_start {
        println!("Start it with:");
        println!("  systemctl enable --now {UNIT_NAME}");
    }
    println!("Inspect logs with: journalctl -u {UNIT_NAME}");

    Ok(())
}

/// Is the unit currently active (`systemctl is-active` == "active")? Returns
/// false on any error (missing systemctl, inactive unit, transient failure) — a
/// conservative default so install never restarts a not-running service.
fn is_unit_active() -> bool {
    let Ok(out) = Command::new("systemctl")
        .args(["is-active", UNIT_NAME])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).trim() == "active"
}

/// Run the `--uninstall` flow. If the unit file is absent, report and exit
/// 3 (idempotent-friendly); otherwise stop/disable/remove + daemon-reload.
pub async fn run_uninstall(_args: Args) -> anyhow::Result<()> {
    validate_platform()?;

    // `--config`/`--auto-start`/`--force` are irrelevant to uninstall.
    let path = unit_path();
    if !path.exists() {
        return Err(exit_err(
            3,
            format!(
                "unit does not exist at {}; nothing to uninstall",
                path.display()
            ),
        ));
    }

    // stop is best-effort: a not-running unit returns a non-zero status.
    if let Err(e) = run_systemctl(&["stop", UNIT_NAME]) {
        warn!("systemctl stop failed (unit may already be stopped): {e}");
    }
    // disable is likewise non-fatal for idempotency.
    if let Err(e) = run_systemctl(&["disable", UNIT_NAME]) {
        warn!("systemctl disable failed (unit may not be enabled): {e}");
    }

    std::fs::remove_file(&path).with_context(|| format!("remove unit file {}", path.display()))?;
    run_systemctl(&["daemon-reload"])?;

    println!("Removed {} ({})", UNIT_NAME, path.display());
    Ok(())
}

fn run_systemctl(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("systemctl")
        .args(args)
        .status()
        .with_context(|| format!("spawn systemctl {}", args.join(" ")))?;
    if !status.success() {
        return Err(anyhow!(
            "systemctl {} failed with status {status}",
            args.join(" ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_inputs(user: &str) -> UnitInputs<'_> {
        UnitInputs {
            binary_abs: Path::new("/usr/local/bin/sebas"),
            config_abs: Path::new("/home/u/cfg.toml"),
            user,
            log_level: "info",
        }
    }

    #[test]
    fn unit_renders_minimal() {
        let s = render_unit(std_inputs("cupen"));
        assert!(s.contains("[Unit]"));
        assert!(s.contains("Description="));
        assert!(s.contains("ExecStart=\"/usr/local/bin/sebas\" watchdog --config \"/home/u/cfg.toml\""));
        assert!(s.contains("Environment=RUST_LOG=info"));
        assert!(s.contains("Restart=on-failure"));
        assert!(s.contains("RestartSec=5"));
        assert!(s.contains("WantedBy=multi-user.target"));
        assert!(!s.contains("WantedBy=default.target"));
    }

    #[test]
    fn unit_writes_user_and_group() {
        let s = render_unit(std_inputs("cupen"));
        assert!(s.contains("User=cupen"));
        assert!(s.contains("Group=cupen"));
    }

    #[test]
    fn unit_runs_watchdog_not_core() {
        let s = render_unit(std_inputs("cupen"));
        assert!(s.contains("watchdog --config"), "ExecStart must run watchdog");
        assert!(!s.contains(" run --config"), "must not run the bare core");
    }

    #[test]
    fn unit_hardening_present_but_no_strict() {
        let s = render_unit(std_inputs("cupen"));
        assert!(s.contains("NoNewPrivileges=true"));
        assert!(s.contains("ProtectSystem=full"));
        assert!(s.contains("ProtectHome=read-only"));
        assert!(s.contains("PrivateTmp=true"));
        assert!(!s.contains("ProtectSystem=strict"), "self-upgrade needs /data writable");
    }

    #[test]
    fn unit_escapes_whitespace_paths() {
        let inputs = UnitInputs {
            binary_abs: Path::new("/opt/my sebas/bin/sebas"),
            config_abs: Path::new("/etc/sebas cfg/sebas.toml"),
            user: "cupen",
            log_level: "info",
        };
        let s = render_unit(inputs);
        // Both args wrapped in a single quoted token.
        assert!(s.contains("ExecStart=\"/opt/my sebas/bin/sebas\" watchdog --config \"/etc/sebas cfg/sebas.toml\""));
    }

    #[test]
    fn unit_idempotent() {
        let a = render_unit(std_inputs("cupen"));
        let b = render_unit(std_inputs("cupen"));
        assert_eq!(a, b, "render must be deterministic");
    }

    fn args_with(user: &str) -> Args {
        Args {
            install: true,
            uninstall: false,
            user: user.into(),
            auto_start: false,
            force: false,
            config: "/etc/sebas.toml".into(),
            log_level: String::new(),
        }
    }

    #[test]
    fn validate_rejects_root_user() {
        let args = args_with("root");
        let err = validate_user(&args.user).unwrap_err();
        assert_eq!(exit_code_of(&err), Some(4));
    }

    #[test]
    fn validate_rejects_empty_user() {
        let args = args_with("");
        let err = validate_user(&args.user).unwrap_err();
        assert_eq!(exit_code_of(&err), Some(4));
    }

    #[test]
    fn validate_rejects_nonexistent_user() {
        // A username that almost certainly doesn't exist on this box.
        let nonce = format!("sebas_nosuch_{}", std::process::id());
        let err = validate_user(&nonce).unwrap_err();
        assert_eq!(exit_code_of(&err), Some(4));
    }
}
