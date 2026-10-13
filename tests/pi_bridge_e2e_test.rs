//! pi 桥文件端到端（make-pi-ride-sebas-provider 1.1/1.2）：
//! 注册表 → `resolve_pi_bridge_spawn_env` → 在 **sebas 私有**
//! `<SEBAS_HOME>/pi-agent/models.json` 落桥 + spawn env 注入。
//!
//! 独立测试二进制的原因（同 `spawn_env_store_authority_test.rs`）：状态引擎
//! 是进程级槽，在独立二进制里初始化内存引擎才不会与其它测试互踩。
//! `state_store::load()` 走 `block_in_place`，用 multi_thread 运行时。
//!
//! 覆盖需求（spec `pi-agent`）：
//! - 「registry entry becomes a pi bridge」：端点来自注册表、密钥经
//!   `${SEBAS_PI_KEY_0}` 插值、落点是 sebas 自己目录；
//! - 「sebas does not spray ANTHROPIC_* env」：spawn env 不含 `ANTHROPIC_*`。

use sebas_dispatch::state_store::{DefaultSelection, PersistedState};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 装一个内存引擎，providers 表含 `anthropic` 协议条目，default_selection
/// 指向它。
fn engine_with_anthropic_provider(name: &str) -> sebas_dispatch::test_engine::EngineGuard {
    let mut state = PersistedState::default();
    let mut item = serde_json::Map::new();
    item.insert(
        "base_url_anthropic".into(),
        serde_json::json!("https://gw.example/anthropic"),
    );
    item.insert("api_key".into(), serde_json::json!("sk-bridge"));
    state.providers.insert(name.into(), item);
    state.default_selection = Some(DefaultSelection::new(name));
    let (_engine, guard) = sebas_dispatch::test_engine::install_fresh_with(state);
    guard
}

/// 注册表 → 桥文件落在 sebas 私有 `<SEBAS_HOME>/pi-agent/models.json`；端点
/// 来自注册表、密钥经占位插值（真值不落盘）；spawn env 含
/// `PI_CODING_AGENT_DIR` + `SEBAS_PI_KEY_0` 真值，**不** 喷 `ANTHROPIC_*`。
#[tokio::test(flavor = "multi_thread")]
async fn bridge_lands_in_sebas_pi_agent_dir_with_registry_endpoint() {
    let _g = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    // SAFETY: ENV_LOCK 全程持有。SEBAS_HOME 钉进一次性目录 → pi-agent 派生在内。
    let saved_home = std::env::var("SEBAS_HOME").ok();
    unsafe { std::env::set_var("SEBAS_HOME", home.path()) };

    let _store = engine_with_anthropic_provider("bridgeprov");

    let (env, diags) = sebas::spawn_env::resolve_pi_bridge_spawn_env(None);
    assert!(diags.is_empty(), "单 provider 无跳过: {diags:?}");

    // 桥文件落在 <SEBAS_HOME>/pi-agent/models.json。
    let bridge = home.path().join("pi-agent").join("models.json");
    let body: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&bridge)
            .unwrap_or_else(|e| panic!("bridge must exist at {}: {e}", bridge.display())),
    )
    .expect("bridge is valid JSON");
    assert_eq!(
        body["providers"]["anthropic"]["baseUrl"], "https://gw.example/anthropic",
        "端点来自注册表: {body}"
    );
    assert_eq!(
        body["providers"]["anthropic"]["apiKey"], "${SEBAS_PI_KEY_0}",
        "密钥经占位插值: {body}"
    );
    let raw = std::fs::read_to_string(&bridge).unwrap();
    assert!(!raw.contains("sk-bridge"), "密钥真值绝不进桥: {raw}");

    // spawn env：PI_CODING_AGENT_DIR 指向 sebas 私有目录 + 密钥真值 env；
    // **不** 喷 ANTHROPIC_*。
    let dir_env = env
        .iter()
        .find(|(k, _)| k == "PI_CODING_AGENT_DIR")
        .map(|(_, v)| v.clone())
        .expect("PI_CODING_AGENT_DIR 注入");
    assert!(
        dir_env.ends_with("pi-agent"),
        "PI_CODING_AGENT_DIR 指向 sebas 私有 pi-agent 目录: {dir_env}"
    );
    assert!(
        env.iter()
            .any(|(k, v)| k == "SEBAS_PI_KEY_0" && v == "sk-bridge"),
        "SEBAS_PI_KEY_0 真值注入: {env:?}"
    );
    assert!(
        !env.iter().any(|(k, _)| k.starts_with("ANTHROPIC_")),
        "桥路径不喷 claude 家族 env: {env:?}"
    );

    // SAFETY: ENV_LOCK held。
    unsafe {
        match saved_home {
            Some(v) => std::env::set_var("SEBAS_HOME", v),
            None => std::env::remove_var("SEBAS_HOME"),
        }
    }
}

/// 空注册表 → 不写桥文件（pi 无凭据，如实报「未配 provider」）；spawn env
/// 仍含 `PI_CODING_AGENT_DIR`（指向空目录，pi 读到空桥 = 无模型），且无
/// `SEBAS_PI_KEY_*`。
#[tokio::test(flavor = "multi_thread")]
async fn empty_registry_writes_no_bridge_and_no_key_env() {
    let _g = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let saved_home = std::env::var("SEBAS_HOME").ok();
    // SAFETY: ENV_LOCK held。
    unsafe { std::env::set_var("SEBAS_HOME", home.path()) };

    // 空 providers + 无 default_selection → Off 形态。
    let (_engine, _guard) =
        sebas_dispatch::test_engine::install_fresh_with(PersistedState::default());

    let (env, diags) = sebas::spawn_env::resolve_pi_bridge_spawn_env(None);
    assert!(diags.is_empty(), "无 provider 不产诊断: {diags:?}");
    assert!(
        !home.path().join("pi-agent").join("models.json").exists(),
        "空注册表不写桥文件"
    );
    assert!(
        !env.iter().any(|(k, _)| k.starts_with("SEBAS_PI_KEY_")),
        "空注册表无密钥 env: {env:?}"
    );
    assert!(
        !env.iter().any(|(k, _)| k.starts_with("ANTHROPIC_")),
        "空注册表也不喷 claude 家族 env: {env:?}"
    );

    // SAFETY: ENV_LOCK held。
    unsafe {
        match saved_home {
            Some(v) => std::env::set_var("SEBAS_HOME", v),
            None => std::env::remove_var("SEBAS_HOME"),
        }
    }
}
