use sebas::config::{AgentConfig, Config};

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
    // fix-pending-queue-liveness 1.2：看门狗阈值默认 600s。
    assert_eq!(cfg.dispatch.turn_stall_timeout, 600);
    assert_eq!(cfg.log.level, "info");
    assert!(cfg.log.file.is_none());
}

/// fix-pending-queue-liveness 1.2：`[dispatch] turn_stall_timeout` 显式配置
/// 生效；`0` = 关闭看门狗（合法值，不是校验错误）。
#[test]
fn turn_stall_timeout_overrides_and_zero_disables() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[dispatch]
turn_stall_timeout = 90
"#;
    let cfg = Config::parse(toml).expect("parse");
    assert_eq!(cfg.dispatch.turn_stall_timeout, 90);

    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[dispatch]
turn_stall_timeout = 0
"#;
    let cfg = Config::parse(toml).expect("0 must parse (guard disabled)");
    assert_eq!(cfg.dispatch.turn_stall_timeout, 0, "0 = watchdog off");
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
    assert!(
        err.contains("acp.claude"),
        "error names the offending table: {err}"
    );
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
    // unify-sebas-home 4.1：media download_dir 缺省 = `<SEBAS_HOME>/cache/
    // downloads`——`SEBAS_HOME` 经同源 expand_tilde 解析，HOME 钉住时落点
    // 仍以其为前缀（该断言保留作为「缺省已展开为绝对路径」的回归）。
    assert!(
        cfg.media
            .download_dir
            .starts_with(&std::env::var("HOME").unwrap_or_default()),
        "default download_dir must expand ~: {}",
        cfg.media.download_dir
    );
}

/// persist-session-map 3.3：`[dispatch] state_file` 键已退休（不留过渡期，
/// design D5）——残留该键的配置以**未知键**解析错误拒绝（deny_unknown_fields，
/// 比「能解析但不生效」的假键诚实）；错误信息点名键名。
#[test]
fn retired_dispatch_state_file_key_is_rejected() {
    let toml = r#"
[feishu]
app_id = ""
app_secret = ""

[dispatch]
state_file = "/tmp/leftover/sessions.json"
"#;
    let err = Config::parse(toml).unwrap_err().to_string();
    assert!(
        err.contains("state_file"),
        "unknown-key error must name the retired key: {err}"
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

[media]
download_dir = "{}/dl"

[log]
file = "{}/logs/sebas.log"
"#,
        dir_toml, dir_toml
    );
    let cfg = Config::parse(&toml).unwrap();
    cfg.validate_runtime().expect("runtime checks pass");
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
    let example =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/config.toml.example");
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

// ── workbench-composer-input-polish 2.1：[acp.agents.<name>] models 覆盖 ──

/// 键缺省（`models` 不出现）= 内置别名表（default/fable/opus/sonnet/haiku）。
#[test]
fn claude_models_key_absent_falls_back_to_builtin_aliases() {
    let toml = r#"
[acp.agents.claude]
driver = "claude"
path = "/bin/cat"
"#;
    let cfg = Config::parse(toml).unwrap();
    let AgentConfig::Claude(c) = cfg.acp.agents.get("claude").unwrap() else {
        panic!("claude agent must parse as Claude config");
    };
    assert!(c.models.is_none(), "key absent must deserialize to None");
    assert_eq!(
        c.resolved_models(),
        vec![
            "default".to_string(),
            "fable".to_string(),
            "opus".to_string(),
            "sonnet".to_string(),
            "haiku".to_string()
        ]
    );
}

/// （add-claude-model-slots 1.2）结构化 `models` table 解析为映射：resolved
/// 词汇 = `default` + 启用档（强弱序，与 TOML 书写序无关）。
#[test]
fn claude_models_structured_table_parses_into_mapping() {
    let toml = r#"
[acp.agents.claude]
driver = "claude"
path = "/bin/cat"

[acp.agents.claude.models]
fable = "glm-5.3"
opus = "opus[1m]"
sonnet = "sonnet"
default = "sonnet"
"#;
    let cfg = Config::parse(toml).unwrap();
    let AgentConfig::Claude(c) = cfg.acp.agents.get("claude").unwrap() else {
        panic!("claude agent must parse as Claude config");
    };
    let m = c.models.as_ref().expect("structured models must parse");
    assert_eq!(m.default_slot, sebas_models::agent::ModelSlot::Sonnet);
    assert_eq!(m.default_model(), Some("sonnet"));
    assert_eq!(m.weakest_model(), Some("sonnet"));
    assert_eq!(
        c.resolved_models(),
        vec!["default", "fable", "opus", "sonnet"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
    );
}

/// （1.2/spec「legacy bare list in config is rejected at parse time」）旧裸
/// 数组形态在解析期报错，且错误点名 `models` 键与期望形态。
#[test]
fn claude_models_legacy_bare_list_is_rejected_naming_the_key() {
    let toml = r#"
[acp.agents.claude]
driver = "claude"
path = "/bin/cat"
models = ["opus", "sonnet"]
"#;
    let err = Config::parse(toml).expect_err("legacy bare list must be rejected");
    let msg = format!("{err}");
    assert!(msg.contains("models"), "错误必须点名 models 键：{msg}");
    assert!(
        msg.contains("结构化") || msg.contains("structured") || msg.contains("table"),
        "错误必须说明期望形态：{msg}"
    );
}

/// （1.2）缺 `default` 键 / default 指向未启用档：解析期拒绝。
#[test]
fn claude_models_missing_or_invalid_default_is_rejected() {
    let missing = r#"
[acp.agents.claude]
driver = "claude"
path = "/bin/cat"

[acp.agents.claude.models]
opus = "o"
sonnet = "s"
"#;
    assert!(Config::parse(missing).is_err(), "缺 default 键必须被拒");

    let dangling = r#"
[acp.agents.claude]
driver = "claude"
path = "/bin/cat"

[acp.agents.claude.models]
opus = "o"
default = "haiku"
"#;
    let err = Config::parse(dangling).expect_err("default 落在未启用档必须被拒");
    assert!(format!("{err}").contains("haiku"), "{err}");
}
