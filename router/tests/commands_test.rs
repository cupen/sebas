use router::commands::{Command, parse_command};

#[test]
fn parses_new() {
    match parse_command("/new") {
        Command::New(p) => assert!(p.is_empty(), "bare /new yields empty prompt"),
        _ => panic!("expected New"),
    }
}

#[test]
fn parses_new_with_prompt() {
    // trailing text 作为初始 prompt（驱动 derive_topic 卡标题 + 引用块）。
    match parse_command("/new 标题啊布拉啊布拉") {
        Command::New(p) => assert_eq!(p, "标题啊布拉啊布拉"),
        _ => panic!("expected New(prompt)"),
    }
    // 多个空格 trim 后保留内部空白（与 splitn(2) 行为一致）。
    match parse_command("/new   hello   world  ") {
        Command::New(p) => assert_eq!(p, "hello   world"),
        _ => panic!("expected New(prompt)"),
    }
}

#[test]
fn parses_switch_with_arg() {
    match parse_command("/switch 3") {
        Command::Switch(3) => {}
        _ => panic!("expected Switch(3)"),
    }
}

#[test]
fn parses_sessions() {
    assert!(matches!(parse_command("/sessions"), Command::Sessions));
}

#[test]
fn parses_help() {
    assert!(matches!(parse_command("/help"), Command::Help));
}

#[test]
fn parses_provider() {
    assert!(matches!(parse_command("/provider"), Command::Provider));
}

#[test]
fn double_slash_escapes_to_prompt() {
    assert_eq!(
        parse_command("//compact"),
        Command::PassThrough("/compact".into())
    );
    assert_eq!(parse_command("/compact"), Command::Compact);
}

#[test]
fn unknown_command_passes_through() {
    assert!(matches!(parse_command("/foo"), Command::PassThrough(_)));
}

#[test]
fn plain_text_is_pass_through() {
    assert_eq!(
        parse_command("hello world"),
        Command::PassThrough("hello world".into())
    );
}

#[test]
fn parse_btw_command() {
    let cmd = parse_command("/btw 顺便问一句");
    assert!(matches!(cmd, Command::Btw(s) if s == "顺便问一句"));
}

#[test]
fn parse_btw_command_empty_text_becomes_pass_through() {
    // /btw with no text falls through to PassThrough
    assert!(matches!(parse_command("/btw"), Command::PassThrough(_)));
}

#[test]
fn parse_settings_alone() {
    assert_eq!(parse_command("/settings"), Command::Settings(None, None));
}

#[test]
fn parse_settings_key_only() {
    assert_eq!(
        parse_command("/settings thinking"),
        Command::Settings(Some("thinking".into()), None)
    );
}

#[test]
fn parse_settings_key_value() {
    assert_eq!(
        parse_command("/settings thinking hide"),
        Command::Settings(Some("thinking".into()), Some("hide".into()))
    );
}

#[test]
fn parse_settings_trims_whitespace() {
    assert_eq!(
        parse_command("  /settings   thinking    show  "),
        Command::Settings(Some("thinking".into()), Some("show".into()))
    );
}

#[test]
fn parse_settings_unknown_key_value_passes_through_value() {
    // We don't validate key names at parse time — validation happens at
    // apply time so the error message can list known keys.
    assert_eq!(
        parse_command("/settings foo bar baz"),
        Command::Settings(Some("foo".into()), Some("bar baz".into()))
    );
}

#[test]
fn parse_upgrade_variants() {
    assert_eq!(
        parse_command("/upgrade"),
        Command::Upgrade {
            dev: false,
            dry_run: false
        }
    );
    assert_eq!(
        parse_command("/upgrade --dry-run"),
        Command::Upgrade {
            dev: false,
            dry_run: true
        }
    );
    assert_eq!(
        parse_command("/upgrade --dev --dry-run"),
        Command::Upgrade {
            dev: true,
            dry_run: true
        }
    );
}

#[test]
fn parse_upgrade_rejects_unknown_flags() {
    assert!(matches!(
        parse_command("/upgrade --force"),
        Command::PassThrough(_)
    ));
}

#[test]
fn parse_rollback() {
    assert_eq!(parse_command("/rollback"), Command::Rollback);
    assert!(matches!(
        parse_command("/rollback now"),
        Command::PassThrough(_)
    ));
}

#[test]
fn parse_upgrade_accepts_bare_dev() {
    assert_eq!(
        parse_command("/upgrade dev"),
        Command::Upgrade {
            dev: true,
            dry_run: false,
        }
    );
    assert_eq!(
        parse_command("/upgrade dev --dry-run"),
        Command::Upgrade {
            dev: true,
            dry_run: true,
        }
    );
}

#[test]
fn parse_watchdog_service_commands() {
    assert_eq!(parse_command("/restart"), Command::Restart);
    assert_eq!(parse_command("/services"), Command::Services);
    assert!(matches!(
        parse_command("/restart now"),
        Command::PassThrough(_)
    ));
    assert!(matches!(
        parse_command("/services all"),
        Command::PassThrough(_)
    ));
}

#[test]
fn parses_goal_as_passthrough() {
    assert_eq!(
        parse_command("/goal 做某件事"),
        Command::PassThrough("/goal 做某件事".into())
    );
}

#[test]
fn parses_model_as_passthrough() {
    assert_eq!(
        parse_command("/model abc"),
        Command::PassThrough("/model abc".into())
    );
}

#[test]
fn parses_model_alone_as_passthrough() {
    assert_eq!(
        parse_command("/model"),
        Command::PassThrough("/model".into())
    );
}

#[test]
fn parses_goal_alone_as_passthrough() {
    assert_eq!(parse_command("/goal"), Command::PassThrough("/goal".into()));
}
