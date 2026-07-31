use sebas::config::Config;

#[test]
fn minimal_config_loads_with_defaults() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"
owner_id = "ou_x"
"#;
    let cfg = Config::parse(toml).expect("parse");
    assert_eq!(cfg.feishu.app_id, "cli_x");
    // defaults filled
    assert_eq!(cfg.acp.claude.idle_kill_secs, 172800);
    assert_eq!(cfg.router.max_concurrent_sessions, 32);
    assert_eq!(cfg.log.level, "info");
    assert!(cfg.log.file.is_none());
}

#[test]
fn missing_required_field_errors() {
    let toml = r#"
[feishu]
app_id = "cli_x"
"#;
    let r = Config::parse(toml);
    assert!(r.is_err());
    let msg = r.unwrap_err().to_string();
    assert!(msg.contains("app_secret"));
}

#[test]
fn owner_id_optional() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"
"#;
    let cfg = Config::parse(toml).expect("parse should succeed without owner_id");
    assert_eq!(cfg.feishu.owner_id, "");
}

#[test]
fn overrides_apply() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"
owner_id = "ou_x"

[acp.claude]
idle_kill_secs = 60

[log]
level = "debug"
"#;
    let cfg = Config::parse(toml).unwrap();
    assert_eq!(cfg.acp.claude.idle_kill_secs, 60);
    assert_eq!(cfg.log.level, "debug");
}

#[test]
fn tilde_expansion_in_default_paths() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"
owner_id = "ou_x"
"#;
    let cfg = Config::parse(toml).unwrap();
    assert!(
        cfg.router
            .state_file
            .starts_with(&std::env::var("HOME").unwrap_or_default())
    );
}

/// env 覆盖测试放在独立的集成测试文件（tests/config_env_test.rs）里——
/// 每个集成测试文件是独立进程，避免 set_var 与本文件的并行断言竞争。

#[test]
fn validate_runtime_accepts_reachable_binary_and_writable_dirs() {
    let dir = std::env::temp_dir().join(format!("sebas-vr-{}", std::process::id()));
    let toml = format!(
        r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[acp.claude]
path = "/bin/cat"

[router]
state_file = "{}/state/sessions.json"

[media]
download_dir = "{}/dl"

[log]
file = "{}/logs/sebas.log"
"#,
        dir.display(),
        dir.display(),
        dir.display()
    );
    let cfg = Config::parse(&toml).unwrap();
    cfg.validate_runtime().expect("runtime checks pass");
    assert!(dir.join("state").is_dir(), "state parent created");
    assert!(dir.join("dl").is_dir(), "download dir created");
    assert!(dir.join("logs").is_dir(), "log parent created");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validate_runtime_rejects_missing_binary() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[acp.claude]
path = "definitely-not-a-real-binary-sebas-test"
"#;
    let cfg = Config::parse(toml).unwrap();
    let err = cfg.validate_runtime().unwrap_err().to_string();
    assert!(
        err.contains("definitely-not-a-real-binary-sebas-test"),
        "error names the binary: {err}"
    );
}

#[test]
fn validate_runtime_rejects_unwritable_dir() {
    let dir = std::env::temp_dir().join(format!("sebas-vr-ro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A regular FILE where a directory should be: create_dir_all must fail.
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let toml = format!(
        r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[acp.claude]
path = "/bin/cat"

[media]
download_dir = "{}/sub"
"#,
        blocker.display()
    );
    let cfg = Config::parse(&toml).unwrap();
    assert!(cfg.validate_runtime().is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
