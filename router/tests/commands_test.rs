use router::commands::{Command, parse_command};

#[test]
fn parses_new() {
    match parse_command("/new") {
        Command::New => {}
        _ => panic!("expected New"),
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
