use router::commands::{parse_command, Command};

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
