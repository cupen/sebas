//! Spawn-time env/args translation from `ProviderMode` + `DefaultProviderForDirect`.
//!
//! Sits between two already-built pieces:
//!
//! - `router::provider_state::ProviderRuntimeState`（bead sebas-63f.3）：runtime 决策
//!   ——「当前走 Off / Direct / Gateway」。
//! - `acp_claude::AgentDriver`（bead sebas-63f.2）：把 `ProviderResolution`
//!   翻成 agent 进程看得懂的 env vars + CLI args（`ClaudeCodeDriver`）。
//!
//! 这里只负责「中间环节」：从 state 拿到语义意图 → 解析上游 URL + 密钥 → 喂
//! 给 driver。对调用方（`acp_spawn_and_activate` / `acp_resume_and_activate`）
//! 暴露一个统一的 [`resolve_spawn_overrides`]，返回 `(extra_env, extra_args)`，
//! 追加到 `claude_args` 上送进 `SessionManager::create_session`。
//!
//! 失败语义：spawn-time 解析失败（gateway URL 没配、named provider 在 overlay
//! 里找不到、api_key_env 没值）一律 `warn!` 然后回退到 `ProviderResolution::Off`。
//! 不让 runtime 配置问题阻断用户发起的 session —— claude 自身允许 run with
//! 它自己找到的 env / config。

use router::provider_state::{ProviderMode, ProviderRuntimeState};
use acp_claude::{AgentDriver, ProviderResolution};
use gateway::config::GatewayConfig;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// 从 `~/.sebas/providers.json` 读单个 provider 的原始 Item（含 `default_model`）。
/// 文件不存在 / JSON 坏 / 名字不在 overrides 里 → `None`（不报错，让上层
/// 决定 graceful fallback 到 `Off`）。
///
/// `default_model` 只在 overlay item 上（gateway `ProviderConfig` 没有这字段，
/// 故意不向 gateway 同步 —— sebas-63f.4 设计决定），所以必须从 overlay 读，
/// 不能从 `gateway_cfg.providers` 拿。
fn read_overlay_item(name: &str) -> Option<Map<String, Value>> {
    let path = crate::provider::overlay_path();
    let raw = std::fs::read_to_string(&path).ok()?;
    #[derive(serde::Deserialize)]
    struct Overlay {
        #[serde(default)]
        providers: HashMap<String, Map<String, Value>>,
        #[serde(default)]
        deleted: Vec<String>,
    }
    let file: Overlay = serde_json::from_str(&raw).ok()?;
    if file.deleted.iter().any(|d| d == name) {
        return None;
    }
    file.providers.get(name).cloned()
}

/// 把 overlay Item 映射到 `ProviderResolution::Direct`。
///
/// 协议选择：优先 `base_url_anthropic`（Anthropic），其次 `base_url_openai`
/// （OpenAI）。两者都缺 → 回退 `Off` + warn。
///
/// 密钥优先级：`api_key` 明文（仅测试用，warn 一条） > `api_key_env` 读 env
/// （env 缺失/空 → 回退 `Off` + warn）。和 `GatewayConfig::resolve_api_keys`
/// 走同一套优先级，行为一致。
fn direct_resolution_from_overlay(
    name: &str,
    item: &Map<String, Value>,
) -> (ProviderResolution, Option<String>) {
    let base_url_anthropic = item
        .get("base_url_anthropic")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let base_url_openai = item
        .get("base_url_openai")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let default_model = item
        .get("default_model")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let (proto, base_url) = if let Some(u) = base_url_anthropic {
        (acp_claude::Protocol::Anthropic, u)
    } else if let Some(u) = base_url_openai {
        (acp_claude::Protocol::OpenAi, u)
    } else {
        tracing::warn!(
            provider = %name,
            "Direct provider has no base_url_anthropic / base_url_openai; falling back to Off"
        );
        return (ProviderResolution::Off, default_model);
    };

    // 密钥：api_key 明文优先（仅测试），否则读 api_key_env。
    let auth_token = if let Some(key) = item
        .get("api_key")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        tracing::warn!(
            provider = %name,
            "Direct provider uses plaintext api_key (overlay-supplied); prefer api_key_env"
        );
        key.to_string()
    } else if let Some(env_var) = item
        .get("api_key_env")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        match std::env::var(env_var) {
            Ok(v) if !v.is_empty() => v,
            _ => {
                tracing::warn!(
                    provider = %name,
                    env_var = %env_var,
                    "Direct provider api_key_env unset/empty; falling back to Off"
                );
                return (ProviderResolution::Off, default_model);
            }
        }
    } else {
        tracing::warn!(
            provider = %name,
            "Direct provider missing both api_key and api_key_env; falling back to Off"
        );
        return (ProviderResolution::Off, default_model);
    };

    (
        ProviderResolution::Direct {
            proto,
            base_url,
            auth_token,
        },
        default_model,
    )
}

/// 从 `GatewayConfig` 派生 `ProviderResolution::Gateway`。
///
/// URL 取 `gateway_cfg.listen`（如 `127.0.0.1:8787`），前面补 `http://`。
/// Auth token 取 `auth_token[0]`（空数组或空字符串 → 不带 auth，但 warn）。
/// 至少 `listen` 必须非空 —— 否则 `Off` + warn。
fn gateway_resolution(cfg: &GatewayConfig) -> (ProviderResolution, Option<String>) {
    let listen = cfg.listen.trim();
    if listen.is_empty() {
        tracing::warn!("ProviderMode::Gateway but gateway.listen is empty; falling back to Off");
        return (ProviderResolution::Off, None);
    }
    let url = format!("http://{listen}");
    let auth_token = cfg.auth_token.first().cloned().unwrap_or_default();
    if auth_token.is_empty() {
        tracing::warn!(
            listen = %listen,
            "ProviderMode::Gateway but gateway.auth_token is empty; agent will call without Bearer/x-api-key"
        );
    }
    (ProviderResolution::Gateway { url, auth_token }, None)
}

/// 解析 `ProviderMode` → `ProviderResolution` + 可选的 `default_model`
/// （直连 provider 才会有；Gateway / Off 一律 `None`）。
///
/// 失败一律 `Off` + warn（永不 panic / 永不 Result Err）—— runtime 配置
/// 错误不应让 claude 启动失败。
pub fn compute_provider_resolution(
    state: &ProviderRuntimeState,
    gateway_cfg: Option<&GatewayConfig>,
) -> (ProviderResolution, Option<String>) {
    match &state.mode {
        ProviderMode::Off => (ProviderResolution::Off, None),
        ProviderMode::Gateway => match gateway_cfg {
            Some(cfg) => gateway_resolution(cfg),
            None => {
                tracing::warn!("ProviderMode::Gateway but no gateway config provided; falling back to Off");
                (ProviderResolution::Off, None)
            }
        },
        ProviderMode::Direct { provider } => {
            // 优先读 overlay（用户 bot 里改的）；overlay 缺则回退到
            // gateway_cfg 里的同名 provider（仅 config.toml 种子，没经过
            // /provider 编辑的 provider 走这条路径）。
            if let Some(item) = read_overlay_item(provider) {
                direct_resolution_from_overlay(provider, &item)
            } else if let Some(cfg) = gateway_cfg
                && let Some(p) = cfg.providers.get(provider)
            {
                // gateway 侧 resolution：已经过 preset 解析与 overlay 合并。
                // 这里用 gateway 自己的 ProviderConfig 反推 Direct 给 agent：
                //   - base_url_anthropic/openai 决定协议；
                //   - api_key_env 读 env，api_key 明文兜底。
                build_direct_from_gateway_config(provider, p)
            } else {
                tracing::warn!(
                    provider = %provider,
                    "Direct provider not found in overlay or gateway config; falling back to Off"
                );
                (ProviderResolution::Off, None)
            }
        }
    }
}

/// 从 `gateway::config::ProviderConfig` 构造 `ProviderResolution::Direct`。
/// 与 `direct_resolution_from_overlay` 语义一致，只是输入形状不同 —— 复用
/// 同一套协议选择 + 密钥优先级，避免两套逻辑漂移。
fn build_direct_from_gateway_config(
    name: &str,
    p: &gateway::config::ProviderConfig,
) -> (ProviderResolution, Option<String>) {
    let (proto, base_url) = if let Some(u) = p.base_url_anthropic.as_deref() {
        (acp_claude::Protocol::Anthropic, u.to_string())
    } else if let Some(u) = p.base_url_openai.as_deref() {
        (acp_claude::Protocol::OpenAi, u.to_string())
    } else {
        tracing::warn!(
            provider = %name,
            "Direct provider missing URLs in gateway config; falling back to Off"
        );
        return (ProviderResolution::Off, None);
    };
    let auth_token = if let Some(env_var) = &p.api_key_env {
        match std::env::var(env_var) {
            Ok(v) if !v.is_empty() => v,
            _ => {
                tracing::warn!(
                    provider = %name,
                    env_var = %env_var,
                    "Direct provider api_key_env unset/empty; falling back to Off"
                );
                return (ProviderResolution::Off, None);
            }
        }
    } else if let Some(plain) = &p.api_key {
        tracing::warn!(
            provider = %name,
            "Direct provider uses plaintext api_key (config.toml-supplied); prefer api_key_env"
        );
        plain.clone()
    } else {
        tracing::warn!(
            provider = %name,
            "Direct provider has neither api_key_env nor api_key; falling back to Off"
        );
        return (ProviderResolution::Off, None);
    };

    (
        ProviderResolution::Direct {
            proto,
            base_url,
            auth_token,
        },
        None, // gateway ProviderConfig 不带 default_model（设计如此）
    )
}

/// 给 agent 进程的额外 env vars + 额外 CLI args。
///
/// 设计：
/// - `extra_env` 来自 driver 的 `resolve_env`（已按 `ProviderMode` 翻译）。
/// - `extra_args` 来自 driver 的 `resolve_args` ∪ `--model <name>`（仅在
///   `default_model` 非空时附加）。
/// - Off / 解析失败：两者都为空 → 与「直接 spawn」的旧行为等价。
pub fn resolve_spawn_overrides(
    driver: &dyn AgentDriver,
    state: &ProviderRuntimeState,
    gateway_cfg: Option<&GatewayConfig>,
) -> (Vec<(String, String)>, Vec<String>) {
    let (resolution, default_model) = compute_provider_resolution(state, gateway_cfg);
    let env = driver.resolve_env(&resolution);
    let mut args = driver.resolve_args(&resolution);
    if let Some(model) = default_model {
        args.push("--model".to_string());
        args.push(model);
    }
    (env, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use router::provider_state::{ProviderMode, ProviderRuntimeState};
    use acp_claude::{ClaudeCodeDriver, Protocol};
    use gateway::config::GatewayConfig;
    use std::sync::Mutex;

    // 串行化所有 env 访问：`SEBAS_GATEWAY_PROVIDER_OVERLAY` 是全局变量，
    // 跨测试并发跑会撞；与 `gateway/src/config.rs::tests` 同惯例。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn driver() -> ClaudeCodeDriver {
        ClaudeCodeDriver
    }

    fn off_state() -> ProviderRuntimeState {
        ProviderRuntimeState::default()
    }

    fn direct_state(name: &str) -> ProviderRuntimeState {
        ProviderRuntimeState {
            mode: ProviderMode::Direct {
                provider: name.into(),
            },
            default_provider_for_direct: Some(name.into()),
        }
    }

    fn gateway_state() -> ProviderRuntimeState {
        ProviderRuntimeState {
            mode: ProviderMode::Gateway,
            default_provider_for_direct: None,
        }
    }

    /// Build a minimal `GatewayConfig` for tests — GatewayConfig has no
    /// `Default` impl (out of scope for this task), so we set the fields
    /// the spawn-env resolver actually touches (`listen`, `auth_token`,
    /// `providers`) and leave the rest at their defaults via `parse`.
    fn test_gateway(listen: &str, auth_token: Vec<String>) -> GatewayConfig {
        let raw = format!(
            r#"
[gateway]
listen = "{listen}"
auth_token = {auth_token:?}
[provider.anthropic]
"#
        );
        GatewayConfig::parse(&raw).expect("test gateway config parses")
    }

    fn write_overlay(dir: &std::path::Path, body: &str) {
        let path = dir.join("providers.json");
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(&path, body).unwrap();
        // SAFETY: ENV_LOCK held across all overlay-touching tests.
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_PROVIDER_OVERLAY", path.to_str().unwrap());
        }
    }

    fn clear_overlay_env() {
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
        }
    }

    // ---- Off ----

    #[test]
    fn off_mode_resolves_to_off_with_no_env_no_args() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_overlay_env();
        let state = off_state();
        let (resolution, model) = compute_provider_resolution(&state, None);
        assert!(matches!(resolution, ProviderResolution::Off));
        assert!(model.is_none());
        let (env, args) = resolve_spawn_overrides(&driver(), &state, None);
        assert!(env.is_empty());
        assert!(args.is_empty());
    }

    // ---- Direct: overlay-supplied ----

    #[test]
    fn direct_overlay_picks_anthropic_url_and_resolves_api_key_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "deepseek": {
                        "preset": "deepseek",
                        "base_url_anthropic": "https://api.deepseek.com/anthropic",
                        "api_key_env": "DEEPSEEK_API_KEY"
                    }
                }
            }"#,
        );
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("DEEPSEEK_API_KEY", "sk-ds-test");
        }
        let state = direct_state("deepseek");
        let (resolution, model) = compute_provider_resolution(&state, None);
        match resolution {
            ProviderResolution::Direct {
                proto,
                base_url,
                auth_token,
            } => {
                assert_eq!(proto, Protocol::Anthropic);
                assert_eq!(base_url, "https://api.deepseek.com/anthropic");
                assert_eq!(auth_token, "sk-ds-test");
            }
            other => panic!("expected Direct, got {other:?}"),
        }
        assert!(model.is_none());
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("DEEPSEEK_API_KEY");
        }
    }

    #[test]
    fn direct_overlay_falls_back_to_openai_url_when_anthropic_missing() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "dashscope": {
                        "base_url_openai": "https://dashscope.aliyuncs.com/compatible-mode/v1",
                        "api_key_env": "DASHSCOPE_API_KEY"
                    }
                }
            }"#,
        );
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("DASHSCOPE_API_KEY", "sk-dash");
        }
        let state = direct_state("dashscope");
        let (resolution, _) = compute_provider_resolution(&state, None);
        match resolution {
            ProviderResolution::Direct {
                proto,
                base_url,
                auth_token,
            } => {
                assert_eq!(proto, Protocol::OpenAi);
                assert_eq!(
                    base_url,
                    "https://dashscope.aliyuncs.com/compatible-mode/v1"
                );
                assert_eq!(auth_token, "sk-dash");
            }
            other => panic!("expected Direct(OpenAI), got {other:?}"),
        }
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("DASHSCOPE_API_KEY");
        }
    }

    #[test]
    fn direct_overlay_emits_model_arg_when_default_model_set() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "deepseek": {
                        "base_url_anthropic": "https://api.deepseek.com/anthropic",
                        "api_key_env": "DEEPSEEK_API_KEY",
                        "default_model": "deepseek-reasoner"
                    }
                }
            }"#,
        );
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("DEEPSEEK_API_KEY", "sk-ds");
        }
        let state = direct_state("deepseek");
        let (env, args) = resolve_spawn_overrides(&driver(), &state, None);
        // 验证 --model 出现在 args 末尾（顺序：resolve_args 返回空 + 我们加 --model）。
        assert_eq!(args, vec!["--model".to_string(), "deepseek-reasoner".to_string()]);
        // env 仍包含 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN，证明 driver 也跑了。
        assert!(env.iter().any(|(k, _)| k == "ANTHROPIC_BASE_URL"));
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("DEEPSEEK_API_KEY");
        }
    }

    #[test]
    fn direct_overlay_plain_api_key_wins_over_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "anthropic": {
                        "base_url_anthropic": "https://api.anthropic.com",
                        "api_key": "sk-anthropic-plain"
                    }
                }
            }"#,
        );
        let state = direct_state("anthropic");
        let (resolution, _) = compute_provider_resolution(&state, None);
        match resolution {
            ProviderResolution::Direct { auth_token, .. } => {
                assert_eq!(auth_token, "sk-anthropic-plain", "api_key 明文应优先");
            }
            other => panic!("expected Direct, got {other:?}"),
        }
    }

    #[test]
    fn direct_overlay_missing_provider_falls_back_to_off() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{ "providers": { "deepseek": { "preset": "deepseek" } } }"#,
        );
        let state = direct_state("nonexistent");
        let (resolution, _) = compute_provider_resolution(&state, None);
        assert!(
            matches!(resolution, ProviderResolution::Off),
            "missing provider must fall back to Off"
        );
    }

    #[test]
    fn direct_overlay_tombstoned_provider_falls_back_to_off() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": { "deepseek": { "preset": "deepseek" } },
                "deleted": ["openai"]
            }"#,
        );
        let state = direct_state("openai");
        let (resolution, _) = compute_provider_resolution(&state, None);
        assert!(matches!(resolution, ProviderResolution::Off));
    }

    #[test]
    fn direct_overlay_api_key_env_unset_falls_back_to_off() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "deepseek": {
                        "base_url_anthropic": "https://api.deepseek.com/anthropic",
                        "api_key_env": "THIS_KEY_IS_NOT_SET_63F8"
                    }
                }
            }"#,
        );
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("THIS_KEY_IS_NOT_SET_63F8");
        }
        let state = direct_state("deepseek");
        let (resolution, _) = compute_provider_resolution(&state, None);
        assert!(matches!(resolution, ProviderResolution::Off));
    }

    #[test]
    fn direct_overlay_no_url_falls_back_to_off() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "weird": {
                        "api_key_env": "WEIRD_KEY"
                    }
                }
            }"#,
        );
        let state = direct_state("weird");
        let (resolution, _) = compute_provider_resolution(&state, None);
        assert!(matches!(resolution, ProviderResolution::Off));
    }

    // ---- Direct: gateway_cfg fallback (no overlay entry) ----

    #[test]
    fn direct_falls_back_to_gateway_cfg_when_overlay_missing() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(dir.path(), r#"{ "providers": {} }"#);
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "sk-anth-gw");
        }
        // 显式构造一份带 `anthropic` provider 的 gateway config，让
        // 「overlay 没找到 → gateway_cfg 兜底」分支被命中。
        let raw = r#"
[gateway]
listen = "127.0.0.1:8787"
auth_token = "x"
[provider.anthropic]
api_key_env = "ANTHROPIC_API_KEY"
"#;
        let cfg = GatewayConfig::parse(raw).expect("test gateway parses");
        let state = direct_state("anthropic");
        let (resolution, _) = compute_provider_resolution(&state, Some(&cfg));
        match resolution {
            ProviderResolution::Direct {
                proto,
                base_url,
                auth_token,
            } => {
                assert_eq!(proto, Protocol::Anthropic);
                assert_eq!(base_url, "https://api.anthropic.com");
                assert_eq!(auth_token, "sk-anth-gw");
            }
            other => panic!("expected Direct via gateway_cfg, got {other:?}"),
        }
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
        }
    }

    // ---- Gateway ----

    #[test]
    fn gateway_mode_uses_http_listen_url_and_first_auth_token() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = test_gateway("127.0.0.1:8787", vec!["sk-gw".to_string()]);
        let state = gateway_state();
        let (resolution, model) = compute_provider_resolution(&state, Some(&cfg));
        match resolution {
            ProviderResolution::Gateway { url, auth_token } => {
                assert_eq!(url, "http://127.0.0.1:8787");
                assert_eq!(auth_token, "sk-gw");
            }
            other => panic!("expected Gateway, got {other:?}"),
        }
        assert!(model.is_none());
    }

    #[test]
    fn gateway_mode_without_cfg_falls_back_to_off() {
        let _g = ENV_LOCK.lock().unwrap();
        let state = gateway_state();
        let (resolution, _) = compute_provider_resolution(&state, None);
        assert!(matches!(resolution, ProviderResolution::Off));
    }

    #[test]
    fn gateway_mode_empty_listen_falls_back_to_off() {
        let _g = ENV_LOCK.lock().unwrap();
        // parse 走一遍拿到合法 cfg，再把 listen 改成空——这样我们精确覆盖
        // `gateway_mode_uses_http_listen_url_and_first_auth_token` 的反向分支。
        let mut cfg = test_gateway("127.0.0.1:8787", vec!["sk-gw".to_string()]);
        cfg.listen = "".to_string();
        let state = gateway_state();
        let (resolution, _) = compute_provider_resolution(&state, Some(&cfg));
        assert!(matches!(resolution, ProviderResolution::Off));
    }

    #[test]
    fn gateway_mode_emits_anthropic_env_via_driver() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = test_gateway("127.0.0.1:8787", vec!["sk-gw".to_string()]);
        let state = gateway_state();
        let (env, args) = resolve_spawn_overrides(&driver(), &state, Some(&cfg));
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_BASE_URL" && v == "http://127.0.0.1:8787"));
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_AUTH_TOKEN" && v == "sk-gw"));
        assert!(args.is_empty());
    }

    // ---- resolve_spawn_overrides integration ----

    #[test]
    fn resolve_spawn_overrides_off_returns_empty() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_overlay_env();
        let state = off_state();
        let (env, args) = resolve_spawn_overrides(&driver(), &state, None);
        assert!(env.is_empty());
        assert!(args.is_empty());
    }

    #[test]
    fn resolve_spawn_overrides_direct_does_not_emit_args_without_default_model() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "deepseek": {
                        "base_url_anthropic": "https://api.deepseek.com/anthropic",
                        "api_key_env": "DEEPSEEK_API_KEY"
                    }
                }
            }"#,
        );
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("DEEPSEEK_API_KEY", "sk-ds");
        }
        let state = direct_state("deepseek");
        let (env, args) = resolve_spawn_overrides(&driver(), &state, None);
        assert!(args.is_empty(), "no default_model → no --model arg");
        assert!(env.iter().any(|(k, _)| k == "ANTHROPIC_BASE_URL"));
        assert!(env.iter().any(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN"));
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("DEEPSEEK_API_KEY");
        }
    }

    // ---- End-to-end (bead sebas-63f.9) ----

    /// 端到端集成测试：从「在 state.json 里改 mode」到「spawn 时拿到的
    /// `(extra_env, extra_args)`」跑一遍完整链路，不真的 fork 进程。
    ///
    /// 为什么需要单独写这个：单测已经覆盖了「每条分支输出什么」，但缺一个
    /// 走「用户改了 state.json → `load()` 读到 → `compute_provider_resolution`
    /// 解析 → `resolve_spawn_overrides` 喂给 driver → 拿到真实 subprocess
    /// env」的贯通路径。这条链路上任何一个 env var 拼错（比如忘了设
    /// `SEBAS_STATE_FILE` 而读了真实 `~/.sebas/state.json`）都会让单测全过
    /// 但生产 spawn 走错分支 —— 这个测试用 tempfile 把两条 env var 重定向
    /// 到临时文件，确保读到的就是我们刚写的。
    #[test]
    fn end_to_end_mode_setting_flows_through_to_spawn_env() {
        let _g = ENV_LOCK.lock().unwrap();

        // 准备两个 tempfile：state.json + providers.json。
        let state_dir = tempfile::tempdir().unwrap();
        let overlay_dir = tempfile::tempdir().unwrap();
        let state_path = state_dir.path().join("state.json");
        let overlay_path = overlay_dir.path().join("providers.json");

        // Overlay 里只放一个 Anthropic 协议的 provider，方便断言 Direct
        // 路径走 Anthropic 分支。
        std::fs::write(
            &overlay_path,
            r#"{
                "providers": {
                    "test_prov": {
                        "preset": "deepseek",
                        "base_url_anthropic": "https://example.test/anthropic",
                        "api_key": "sk-test-direct"
                    }
                }
            }"#,
        )
        .unwrap();

        // 重定向两条全局 env var 到 tempfile，让 production code 读到我们
        // 写的内容（而不是真的 ~/.sebas/*）。
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("SEBAS_STATE_FILE", state_path.to_str().unwrap());
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                overlay_path.to_str().unwrap(),
            );
        }

        // --- Scenario A: Off → Off，无 env 无 args ---
        std::fs::write(
            &state_path,
            r#"{"mode":{"kind":"off"},"default_provider_for_direct":null}"#,
        )
        .unwrap();
        let st = router::provider_state::load();
        let (env, args) = resolve_spawn_overrides(&driver(), &st, None);
        assert!(matches!(
            compute_provider_resolution(&st, None).0,
            ProviderResolution::Off
        ));
        assert!(env.is_empty(), "Off 不应给 driver 任何 env");
        assert!(args.is_empty(), "Off 不应给 driver 任何 args");

        // --- Scenario B: Direct + overlay 命中 → Direct(Anthropic) ---
        std::fs::write(
            &state_path,
            r#"{"mode":{"kind":"direct","provider":"test_prov"},"default_provider_for_direct":"test_prov"}"#,
        )
        .unwrap();
        let st = router::provider_state::load();
        let (env, args) = resolve_spawn_overrides(&driver(), &st, None);
        match compute_provider_resolution(&st, None).0 {
            ProviderResolution::Direct {
                proto,
                base_url,
                auth_token,
            } => {
                assert_eq!(proto, Protocol::Anthropic);
                assert_eq!(base_url, "https://example.test/anthropic");
                assert_eq!(auth_token, "sk-test-direct");
            }
            other => panic!("expected Direct, got {other:?}"),
        }
        // driver 必须把 Direct 翻译成 ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN，
        // 并把这两个变量送给 subprocess。args 空因为 overlay 里没设 default_model。
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_BASE_URL"
            && v == "https://example.test/anthropic"));
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_AUTH_TOKEN"
            && v == "sk-test-direct"));
        assert!(args.is_empty(), "no default_model → no --model args");

        // --- Scenario C: Gateway → Gateway ---
        std::fs::write(
            &state_path,
            r#"{"mode":{"kind":"gateway"},"default_provider_for_direct":null}"#,
        )
        .unwrap();
        let st = router::provider_state::load();
        let cfg = test_gateway("127.0.0.1:8888", vec!["sk-gw-test".to_string()]);
        match compute_provider_resolution(&st, Some(&cfg)).0 {
            ProviderResolution::Gateway { url, auth_token } => {
                assert_eq!(url, "http://127.0.0.1:8888");
                assert_eq!(auth_token, "sk-gw-test");
            }
            other => panic!("expected Gateway, got {other:?}"),
        }

        // --- Scenario D: Direct + 不存在的 provider → Off（不 panic，warn 已记录）---
        std::fs::write(
            &state_path,
            r#"{"mode":{"kind":"direct","provider":"nonexistent"},"default_provider_for_direct":null}"#,
        )
        .unwrap();
        let st = router::provider_state::load();
        let (env, args) = resolve_spawn_overrides(&driver(), &st, None);
        assert!(
            matches!(
                compute_provider_resolution(&st, None).0,
                ProviderResolution::Off
            ),
            "missing Direct provider 必须回退到 Off（runtime 配置错不能让 spawn 崩）"
        );
        assert!(env.is_empty(), "fallback 到 Off 不应给 driver 任何 env");
        assert!(args.is_empty(), "fallback 到 Off 不应给 driver 任何 args");

        // 清理 env var，避免污染后续测试 / CI 环境。
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("SEBAS_STATE_FILE");
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
        }
    }
}
