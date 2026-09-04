use sebas_router::cards::{CardConfig, ThinkingDisplay};
use sebas_router::settings::{load_settings, save_settings, settings_path};

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
    let cfg = CardConfig {
        thinking: ThinkingDisplay::Hide,
        ..CardConfig::default()
    };
    save_settings(&path, &cfg).unwrap();
    let loaded = load_settings(&path).unwrap().expect("file exists");
    assert_eq!(loaded.thinking, ThinkingDisplay::Hide);
}

#[test]
fn load_missing_returns_none_so_caller_keeps_toml() {
    // 文件不存在 → Ok(None) 而不是默认 CardConfig。
    // 调用方（run.rs）见到 None 时回落 TOML [card]，避免每次启动把
    // TOML 调好的 theme_color / max_user_text_chars / thinking 默默抹平。
    let dir = tempdir();
    let path = dir.join("missing.json");
    let loaded = load_settings(&path).unwrap();
    assert!(
        loaded.is_none(),
        "missing file must signal 'use TOML', got {loaded:?}"
    );
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

/// TOML bootstrap 回归保护：settings.json 缺失时 TOML 的 CardConfig 字段
/// 必须保留原值，不得被 `CardConfig::default()` 抹平。
/// 这里模拟 run.rs 的合并：load_settings Ok(None) → 用 TOML 配置。
#[test]
fn toml_cardconfig_survives_when_settings_json_absent() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("sebas-toml-bootstrap-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json"); // 不创建 → 模拟首次启动
    let toml_cfg = CardConfig {
        theme_color: "orange".into(),
        max_user_text_chars: 1234,
        max_tool_output_chars: 567,
        fold_long_output: false,
        thinking: ThinkingDisplay::Hide,
    };
    let merged = match load_settings(&path).unwrap() {
        Some(s) => s,
        None => toml_cfg.clone(),
    };
    assert_eq!(
        merged, toml_cfg,
        "TOML bootstrap must not be overwritten by defaults"
    );
}

/// settings.json 写盘后必须 0600，防止多用户主机上其他用户读到 bot 的
/// 卡片配置。仅 Unix —— Windows 没 POSIX 权限位概念。
#[cfg(unix)]
#[test]
fn save_settings_chmods_0600_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("settings.json");
    save_settings(&path, &CardConfig::default()).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "expected 0600, got {mode:o}");
}

fn tempdir() -> std::path::PathBuf {
    // 把测试目录按 process::id() + 计数器隔离，避开 cargo test 的并行 race。
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("sebas-settings-test-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&p).unwrap();
    p
}
