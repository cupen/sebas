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
    assert_eq!(cfg.acp.idle_kill_for("claude"), 172800);
    assert_eq!(cfg.dispatch.max_concurrent_sessions, 32);
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

[acp.agents.claude]
driver = "claude"
path = "/bin/cat"
idle_kill_secs = 60

[log]
level = "debug"
"#;
    let cfg = Config::parse(toml).unwrap();
    assert_eq!(cfg.acp.idle_kill_for("claude"), 60);
    assert_eq!(cfg.log.level, "debug");
}

#[test]
fn legacy_claude_block_fails_parse() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"
owner_id = "ou_x"

[acp.claude]
path = "/bin/cat"
idle_kill_secs = 60
"#;
    let err = Config::parse(toml)
        .expect_err("legacy [acp.claude] must be rejected at parse time")
        .to_string();
    assert!(err.contains("acp.claude"), "error names the offending table: {err}");
}

#[test]
fn single_acp_agent_gets_implicit_default() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[acp.agents.gemini]
driver = "acp"
command = ["gemini", "--acp"]
"#;
    let cfg = Config::parse(toml).unwrap();
    assert_eq!(cfg.acp.default.as_deref(), Some("gemini"));
    assert_eq!(
        cfg.acp.command_for("gemini"),
        Some(vec!["gemini".to_string(), "--acp".to_string()])
    );
}

#[test]
fn unknown_driver_tag_errors() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[acp.agents.foo]
driver = "foobar"
command = ["foo"]
"#;
    assert!(Config::parse(toml).is_err());
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
        cfg.dispatch
            .state_file
            .starts_with(&std::env::var("HOME").unwrap_or_default())
    );
}

/// env 覆盖测试放在独立的集成测试文件（tests/config_env_test.rs）里——
/// 每个集成测试文件是独立进程，避免 set_var 与本文件的并行断言竞争。

#[test]
fn validate_runtime_accepts_reachable_binary_and_writable_dirs() {
    let dir = std::env::temp_dir().join(format!("sebas-vr-{}", std::process::id()));
    // Windows 路径含反斜杠，嵌入 TOML 字符串必须转义。
    let dir_toml = dir.display().to_string().replace('\\', "\\\\");
    let bin = if cfg!(windows) { "cmd.exe" } else { "/bin/cat" };
    let toml = format!(
        r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[acp.agents.claude]
driver = "claude"
path = "{bin}"

[dispatch]
state_file = "{}/state/sessions.json"

[media]
download_dir = "{}/dl"

[log]
file = "{}/logs/sebas.log"
"#,
        dir_toml, dir_toml, dir_toml
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

[acp.agents.claude]
driver = "claude"
path = "definitely-not-a-real-binary-sebas-test"
"#;
    let cfg = Config::parse(toml).unwrap();
    let err = cfg.validate_runtime().unwrap_err().to_string();
    assert!(
        err.contains("definitely-not-a-real-binary-sebas-test"),
        "error names the binary: {err}"
    );
}

/// feishu 可选（sebas-2ty）回归：[feishu] 段存在且凭据为**显式空串**（仓库
/// 自带 config 的写法）必须解析成功并视为未接入——旧版把 app_id 当必填，
/// 启动即报 "feishu.app_id is required"。
#[test]
fn feishu_section_with_empty_credentials_is_optional() {
    let toml = r#"
[feishu]
app_id = ""
app_secret = ""
"#;
    let cfg = Config::parse(toml).expect("显式空串凭据 = 不接入飞书，必须可解析");
    assert!(!cfg.feishu.is_enabled(), "空串凭据应视为未启用");
}

/// 仓库自带的 config.toml.example 与用户实际 config/config.toml 同构
/// （feishu 凭据空串），作为真实工件兜底，防必填校验回归。
#[test]
fn shipped_config_example_parses_without_feishu_required() {
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/config.toml.example");
    let raw = std::fs::read_to_string(&example).expect("config.toml.example 应随仓库存在");
    let cfg = Config::parse(&raw).expect("仓库自带示例配置必须可解析（feishu 可选）");
    if cfg.feishu.app_id.is_empty() && cfg.feishu.app_secret.is_empty() {
        assert!(!cfg.feishu.is_enabled(), "空凭据示例应视为未接入飞书");
    }
}

#[test]
fn validate_runtime_rejects_unwritable_dir() {
    let dir = std::env::temp_dir().join(format!("sebas-vr-ro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A regular FILE where a directory should be: create_dir_all must fail.
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let blocker_toml = blocker.display().to_string().replace('\\', "\\\\");
    let toml = format!(
        r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[acp.agents.claude]
driver = "claude"
path = "/bin/cat"

[media]
download_dir = "{}/sub"
"#,
        blocker_toml
    );
    let cfg = Config::parse(&toml).unwrap();
    assert!(cfg.validate_runtime().is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
