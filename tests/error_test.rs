use sebas::error::SebasError;

#[test]
fn error_display_includes_context() {
    let e = SebasError::Config("missing app_id".into());
    assert_eq!(e.to_string(), "config error: missing app_id");
}

#[test]
fn error_from_io() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
    let e: SebasError = io_err.into();
    assert!(matches!(e, SebasError::Io(_)));
}
