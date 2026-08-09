use feishu::cards::{CardConfig, ThinkingDisplay};
use router::settings::{load_settings, save_settings, settings_path};

#[test]
fn settings_path_under_home_sebas_dir() {
    let p = settings_path();
    let s = p.to_string_lossy();
    assert!(s.contains(".sebas"), "expected .sebas dir, got {s}");
    assert!(s.ends_with("settings.json"), "got {s}");
}

#[test]
fn save_then_load_round_trips() {
    let dir = tempdir();
    let path = dir.join("settings.json");
    let cfg = CardConfig { thinking: ThinkingDisplay::Hide, ..CardConfig::default() };
    save_settings(&path, &cfg).unwrap();
    let loaded = load_settings(&path).unwrap();
    assert_eq!(loaded.thinking, ThinkingDisplay::Hide);
}

#[test]
fn load_missing_returns_default() {
    let dir = tempdir();
    let path = dir.join("missing.json");
    let loaded = load_settings(&path).unwrap();
    assert_eq!(loaded, CardConfig::default());
}

#[test]
fn load_malformed_returns_error() {
    let dir = tempdir();
    let path = dir.join("bad.json");
    std::fs::write(&path, "{not json").unwrap();
    assert!(load_settings(&path).is_err());
}

#[test]
fn save_writes_pretty_json() {
    let dir = tempdir();
    let path = dir.join("settings.json");
    save_settings(&path, &CardConfig::default()).unwrap();
    let s = std::fs::read_to_string(&path).unwrap();
    assert!(s.contains('\n'), "expected pretty-printed JSON, got: {s}");
}

fn tempdir() -> std::path::PathBuf {
    // 把测试目录按 process::id() + 计数器隔离，避开 cargo test 的并行 race。
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "sebas-settings-test-{}-{}",
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}