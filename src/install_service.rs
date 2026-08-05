//! `sebas install-service` — render + install a systemd user or system unit.
//!
//! Pure functions live at the top (`render_unit`, `UnitInputs`, `Scope`).
//! The `run` entry point handles argument validation, FS effects, and the
//! `systemctl` invocations. macOS is unsupported for the systemd path; we
//! exit 6 with a hint to hand-write a launchd plist.
//!
//! Exit codes (from the brief):
//!   2 = scope missing (`--user` and `--system` both absent)
//!   3 = unit file already exists at `unit_path(scope)` (and `!force`)
//!   4 = EUID/flag conflict (EUID==0 with `--user`, or `--run-as` with `--user`)
//!   5 = `--config` path missing or not absolute
//!   6 = unsupported platform (macOS)

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow};
use tracing::{info, warn};

/// Where the unit file will be written / is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    System,
}

#[derive(Debug)]
pub struct InstallServiceArgs {
    /// Exactly one of these must be `true`.
    pub user: bool,
    pub system: bool,
    pub auto_start: bool,
    pub force: bool,
    pub run_as: Option<String>,
    pub config: String,
}

/// Inputs to the pure renderer. Held by reference so tests can pass
/// `Path::new("…")` literals without allocation.
pub struct UnitInputs<'a> {
    pub binary_abs: &'a Path,
    pub config_abs: &'a Path,
    /// Only honored under `Scope::System`; the `run` validator rejects it
    /// under `Scope::User`.
    pub run_as: Option<&'a str>,
    pub log_level: &'a str,
}

const UNIT_NAME: &str = "sebas.service";
const DESCRIPTION: &str = "sebas — bridge Feishu ↔ Claude Code via ACP";

/// Render the full systemd unit file text. Pure: no FS, no system calls.
/// The four unit tests in this module are the contract.
pub fn render_unit(scope: Scope, inputs: UnitInputs<'_>) -> String {
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
    if let Some(user) = inputs.run_as
        && matches!(scope, Scope::System)
    {
        s.push_str(&format!("User={user}\n"));
        s.push_str(&format!("Group={user}\n"));
    }
    s.push_str(&format!("Environment=RUST_LOG={}\n", inputs.log_level));
    s.push_str(&format!(
        "ExecStart={} run --config {}\n",
        inputs.binary_abs.display(),
        inputs.config_abs.display(),
    ));
    s.push_str("Restart=on-failure\n");
    s.push_str("RestartSec=5\n");
    s.push('\n');

    // [Install]
    s.push_str("[Install]\n");
    let wanted_by = match scope {
        Scope::User => "default.target",
        Scope::System => "multi-user.target",
    };
    s.push_str(&format!("WantedBy={wanted_by}\n"));

    s
}

/// Resolve the unit-file path for the given scope. Returns an error if the
/// `XDG_CONFIG_HOME`/`HOME` env or the system path can't be computed.
pub fn unit_path(scope: Scope) -> anyhow::Result<PathBuf> {
    match scope {
        Scope::User => {
            // XDG_CONFIG_HOME, falling back to $HOME/.config
            let base = std::env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME")
                        .filter(|v| !v.is_empty())
                        .map(|h| PathBuf::from(h).join(".config"))
                })
                .ok_or_else(|| {
                    anyhow!("could not resolve user config dir (set HOME or XDG_CONFIG_HOME)")
                })?;
            Ok(base.join("systemd").join("user").join(UNIT_NAME))
        }
        Scope::System => Ok(PathBuf::from("/etc/systemd/system").join(UNIT_NAME)),
    }
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
        // Non-Unix platforms (Windows) have no EUID concept. `run()` exits 6
        // before this is ever consulted; return a non-zero placeholder so the
        // system-scope root check would fail loudly if that ever changes.
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

/// Print the usage line for `install-service` to stderr.
fn print_usage() {
    eprintln!(
        "usage: sebas install-service (--user|--system) [--auto-start] [--run-as USER] \
         [--force] [--config PATH]"
    );
}

/// Run the install-service flow. See brief section "Behavior (run)" for
/// the step-by-step. Errors are returned as `anyhow::Error`; the caller
/// (`main.rs`) maps the `.exit_code` downcast to a process exit.
pub async fn run(args: InstallServiceArgs) -> anyhow::Result<()> {
    // 0. systemd only exists on Linux; macOS has no systemd either. Exit 6.
    if cfg!(not(unix)) || cfg!(target_os = "macos") {
        return Err(exit_err(
            6,
            "systemd is not available on this platform; use `sebas run` directly",
        ));
    }

    // 1. Scope must be exactly one of --user / --system.
    let scope = match (args.user, args.system) {
        (true, false) => Scope::User,
        (false, true) => Scope::System,
        (true, true) => {
            print_usage();
            return Err(exit_err(2, "--user and --system are mutually exclusive"));
        }
        (false, false) => {
            print_usage();
            return Err(exit_err(2, "specify exactly one of --user or --system"));
        }
    };

    // 2. --run-as is only meaningful under --system.
    if matches!(scope, Scope::User) && args.run_as.is_some() {
        return Err(exit_err(4, "--run-as is only valid with --system"));
    }

    // 4. EUID rules.
    let euid = current_euid();
    if matches!(scope, Scope::User) && euid == 0 {
        return Err(exit_err(
            4,
            "user scope with EUID 0 is not supported; use --system or drop privileges first",
        ));
    }
    if matches!(scope, Scope::System) && euid != 0 {
        return Err(exit_err(
            4,
            "system scope requires root (use sudo / run as root)",
        ));
    }

    // 5. --config must be absolute and exist.
    let config_path = PathBuf::from(&args.config);
    if !config_path.is_absolute() {
        return Err(exit_err(
            5,
            format!("--config must be an absolute path, got: {}", args.config),
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

    // 3. Binary path: `current_exe()` and refuse if not absolute. It always
    //    is on Linux/macOS, but be defensive.
    let binary_abs = std::env::current_exe().context("resolve current_exe()")?;
    if !binary_abs.is_absolute() {
        return Err(exit_err(
            4,
            format!("current_exe() is not absolute: {}", binary_abs.display()),
        ));
    }

    // 6. Unit path + overwrite check.
    let path = unit_path(scope)?;
    if path.exists() && !args.force {
        return Err(exit_err(
            3,
            format!(
                "unit already exists at {}; use --force to overwrite",
                path.display()
            ),
        ));
    }

    // 7. Make the parent dir (best-effort; write() will catch real errors).
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create_dir_all({})", parent.display()))?;
    }

    // 8. Render + write.
    let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    let body = render_unit(
        scope,
        UnitInputs {
            binary_abs: &binary_abs,
            config_abs: &config_path,
            run_as: args.run_as.as_deref(),
            log_level: &log_level,
        },
    );
    std::fs::write(&path, &body).with_context(|| format!("write unit file {}", path.display()))?;
    info!(path = %path.display(), "unit written");

    // 9. daemon-reload
    run_systemctl(scope, &["daemon-reload"])?;

    // 10. Optionally enable+start
    if args.auto_start {
        run_systemctl(scope, &["enable", UNIT_NAME])?;
        run_systemctl(scope, &["start", UNIT_NAME])?;
    }

    // 11. Friendly summary.
    if matches!(scope, Scope::System) && args.run_as.is_none() {
        warn!("system unit runs as root; consider --run-as <user> to drop privileges");
    }
    let journal_flag = match scope {
        Scope::User => "--user",
        Scope::System => "",
    };
    let journal_flag = journal_flag.trim();
    println!("Installed {} to {}", UNIT_NAME, path.display());
    if !args.auto_start {
        println!("Start it with:");
        if journal_flag.is_empty() {
            println!("  systemctl enable --now {UNIT_NAME}");
        } else {
            println!("  systemctl {journal_flag} enable --now {UNIT_NAME}");
        }
    }
    let journal_cmd = if journal_flag.is_empty() {
        format!("journalctl -u {UNIT_NAME}")
    } else {
        format!("journalctl -u {UNIT_NAME} {journal_flag}")
    };
    println!("Inspect logs with: {journal_cmd}");

    Ok(())
}

fn run_systemctl(scope: Scope, args: &[&str]) -> anyhow::Result<()> {
    let mut cmd = Command::new("systemctl");
    if matches!(scope, Scope::User) {
        cmd.arg("--user");
    }
    cmd.args(args);
    let status = cmd
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

    fn std_inputs<'a>(run_as: Option<&'a str>) -> UnitInputs<'a> {
        UnitInputs {
            binary_abs: Path::new("/usr/local/bin/sebas"),
            config_abs: Path::new("/home/u/cfg.toml"),
            run_as,
            log_level: "info",
        }
    }

    #[test]
    fn unit_user_renders_minimal() {
        let s = render_unit(Scope::User, std_inputs(None));
        assert!(s.contains("[Unit]"));
        assert!(s.contains("Description="));
        assert!(s.contains("ExecStart=/usr/local/bin/sebas run --config /home/u/cfg.toml"));
        assert!(!s.contains("User="));
        assert!(!s.contains("Group="));
        assert!(s.contains("Environment=RUST_LOG=info"));
        assert!(s.contains("Restart=on-failure"));
        assert!(s.contains("RestartSec=5"));
        assert!(s.contains("WantedBy=default.target"));
    }

    #[test]
    fn unit_system_no_run_as_warns_root() {
        // render_unit itself is pure; the "warn" lives in `run`. Just
        // confirm the unit omits User=/Group= and targets multi-user.
        let s = render_unit(Scope::System, std_inputs(None));
        assert!(!s.contains("User="));
        assert!(!s.contains("Group="));
        assert!(s.contains("WantedBy=multi-user.target"));
        assert!(!s.contains("WantedBy=default.target"));
    }

    #[test]
    fn unit_system_run_as_writes_user_and_group() {
        let s = render_unit(Scope::System, std_inputs(Some("cupen")));
        assert!(s.contains("User=cupen"));
        assert!(s.contains("Group=cupen"));
        assert!(s.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn unit_idempotent() {
        let a = render_unit(Scope::User, std_inputs(None));
        let b = render_unit(Scope::User, std_inputs(None));
        assert_eq!(a, b, "render must be deterministic");
    }

    #[test]
    fn user_scope_ignores_run_as_in_render() {
        // run_as under User must not emit User=/Group=; the run()
        // validator rejects the args combination before we get here.
        let s = render_unit(Scope::User, std_inputs(Some("ignored")));
        assert!(!s.contains("User="));
        assert!(!s.contains("Group="));
    }

    /// Print the rendered user unit so reports/eyeballs can verify the
    /// systemd output. Run with `--ignored --nocapture`.
    #[test]
    #[ignore = "manual: prints the rendered user unit"]
    fn print_user_unit_for_report() {
        let s = render_unit(
            Scope::User,
            UnitInputs {
                binary_abs: Path::new("/usr/local/bin/sebas"),
                config_abs: Path::new("/home/bot/cfg.toml"),
                run_as: None,
                log_level: "info",
            },
        );
        println!("---BEGIN RENDERED USER UNIT---\n{s}---END RENDERED USER UNIT---");
    }
}
