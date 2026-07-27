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
    assert!(matches!(cfg.log.file, None));
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
