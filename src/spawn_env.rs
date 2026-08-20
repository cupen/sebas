//! Spawn-time env/args translation from `ProviderMode` + `DefaultSelection`.
//!
//! Sits between two already-built pieces:
//!
//! - `router::provider_state::ProviderRuntimeState`（bead sebas-63f.3）：runtime 决策
//!   ——「当前走 Off / Direct / Gateway」。
//! - `acp_claude::ClaudeCodeDriver`（bead sebas-63f.2）：把 `ProviderResolution`
//!   翻成 agent 进程看得懂的 env vars + CLI args。
//!
//! 这里只负责「中间环节」：从 state 拿到语义意图 → 解析上游 URL + 密钥 → 喂
//! 给 driver。对调用方（`acp_spawn_and_activate` / `acp_resume_and_activate`）
//! 暴露一个统一的 [`resolve_spawn_overrides`]，返回 `(extra_env, extra_args)`，
//! 追加到 `claude_args` 上送进 `SessionManager::create_session`。
//!
//! 失败语义（spec 2026-08-17 §2.2）：spawn-time 解析失败（gateway URL 没配、
//! named provider 在 overlay 里找不到、api_key_env 没值）一律返回
//! `ProviderResolution::Error { reason }`。driver 把这个变体翻译成单条
//! `SEBAS_PROVIDER_ERROR=<reason>` env var，spawn wrapper（`session_boot`）
//! 看到这条 var 就立刻 `print` + `exit(1)`，不真的去 fork claude 子进程。
//! 旧行为是回退到 `ProviderResolution::Off`，让 claude 用自己 env / config
//! —— 用户看到"启动了但啥都没发生"时无法定位是 sebas 的问题还是 claude
//! 自己环境的问题。新行为把错误直接喂给用户。

use acp_claude::{ClaudeCodeDriver, ProviderResolution};
use gateway::config::GatewayConfig;
use router::provider_state::{ProviderMode, ProviderRuntimeState};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// 从 `~/.sebas/providers.json`（legacy overlay）或 `~/.sebas/state.json`
/// （spec 2026-08-17 §2.6 合并后的统一持久化文件）读单个 provider 的原始
/// Item（含 `default_model`）。文件不存在 / JSON 坏 / 名字不在 overrides
/// 里 / 已 tombstone → `None`（不报错，让上层决定 graceful fallback 到 `Off`）。
///
/// 优先读 legacy overlay（兼容旧用户）；overlay 不存在时回退到 unified
/// `state.json`（新部署走 state_store 后，providers.json 已被迁移 + 删除）。
///
/// `default_model` 只在 overlay item 上（gateway `ProviderConfig` 没有这字段，
/// 故意不向 gateway 同步 —— sebas-63f.4 设计决定），所以必须从 overlay 读，
/// 不能从 `gateway_cfg.providers` 拿。
fn read_overlay_item(name: &str) -> Option<Map<String, Value>> {
    // 优先：legacy overlay（spec §2.6 前的旧路径）。
    let overlay_path = crate::provider::overlay_path();
    if overlay_path.exists() {
        let raw = std::fs::read_to_string(&overlay_path).ok()?;
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
        if let Some(item) = file.providers.get(name).cloned() {
            return Some(item);
        }
    }
    // 回退：unified state.json（spec §2.6 后的新路径；旧用户首次 load 时
    // overlay 已被迁移 + 删除）。
    let state = router::state_store::load();
    if state.deleted.iter().any(|d| d == name) {
        return None;
    }
    state.providers.get(name).cloned()
}

/// 把 overlay Item 映射到 `ProviderResolution::Direct`。
///
/// 协议选择（spec 2026-08-17 §2.4 — UI 在 `/provider` 详情面板里暴露的
/// 「协议」radio 写到这里）：
/// - `"anthropic"` → 强制走 `base_url_anthropic`；缺失 → `Off` + warn。
/// - `"openai"`    → 强制走 `base_url_openai`；缺失 → `Off` + warn。
/// - `"auto"` / 缺省 → 保持旧约定：优先 `base_url_anthropic`，其次
///   `base_url_openai`。两者都缺 → `Off` + warn。
///
/// 旧 overlay 没 `protocol` 字段的 provider 走「auto」分支，行为不变
/// （向后兼容 spec §2.4）。
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
    // 协议选择：UI 在详情面板的 radio。缺省 = "auto" = 旧约定（Anthropic 优先），
    // 不破坏现有用户的默认行为（spec 2026-08-17 §2.4）。
    let protocol = item
        .get("protocol")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("auto");

    let (proto, base_url) = match protocol {
        "anthropic" => match base_url_anthropic {
            Some(u) => (acp_claude::AgentProtocol::Anthropic, u),
            None => {
                let reason = format!(
                    "direct provider '{name}' has explicit protocol=anthropic but no base_url_anthropic"
                );
                tracing::warn!(
                    provider = %name,
                    "Direct provider 显式 protocol=anthropic 但缺 base_url_anthropic; aborting spawn"
                );
                return (ProviderResolution::Error { reason }, default_model);
            }
        },
        "openai" => match base_url_openai {
            Some(u) => (acp_claude::AgentProtocol::OpenAi, u),
            None => {
                let reason = format!(
                    "direct provider '{name}' has explicit protocol=openai but no base_url_openai"
                );
                tracing::warn!(
                    provider = %name,
                    "Direct provider 显式 protocol=openai 但缺 base_url_openai; aborting spawn"
                );
                return (ProviderResolution::Error { reason }, default_model);
            }
        },
        // "auto" 或未知值：旧约定（anthropic > openai）。
        _ => {
            if let Some(u) = base_url_anthropic {
                (acp_claude::AgentProtocol::Anthropic, u)
            } else if let Some(u) = base_url_openai {
                (acp_claude::AgentProtocol::OpenAi, u)
            } else {
                let reason = format!(
                    "direct provider '{name}' has no base_url_anthropic or base_url_openai"
                );
                tracing::warn!(
                    provider = %name,
                    "Direct provider has no base_url_anthropic / base_url_openai; aborting spawn"
                );
                return (ProviderResolution::Error { reason }, default_model);
            }
        }
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
                let reason =
                    format!("direct provider '{name}' api_key_env '{env_var}' is unset or empty");
                tracing::warn!(
                    provider = %name,
                    env_var = %env_var,
                    "Direct provider api_key_env unset/empty; aborting spawn"
                );
                return (ProviderResolution::Error { reason }, default_model);
            }
        }
    } else {
        let reason = format!("direct provider '{name}' has neither api_key nor api_key_env");
        tracing::warn!(
            provider = %name,
            "Direct provider missing both api_key and api_key_env; aborting spawn"
        );
        return (ProviderResolution::Error { reason }, default_model);
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
        let reason = "ProviderMode::Gateway but gateway.listen is empty in config".to_string();
        tracing::warn!("ProviderMode::Gateway but gateway.listen is empty; aborting spawn");
        return (ProviderResolution::Error { reason }, None);
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

/// 解析 `ProviderMode` + `DefaultSelection` → `ProviderResolution` + 可选的
/// `default_model`（spawn 时追加 `--model <id>`，仅 Direct / Off-with-default
/// 模式下生效；Gateway 一律 `None`）。
///
/// spec 2026-08-17 §2.8 三处决策点：
///
/// 1. **Off + default_selection 已设** → 视为隐式 Direct，按
///    `default_selection.provider` 解析（同 Direct{...} 路径）。这是
///    spec §2.8 的新行为：用户没显式切 Direct 但已经「设为默认（DIRECT）」时
///    也应该让默认 provider 生效。
/// 2. **Off + default_selection 未设** → `ProviderResolution::Off`（保持
///    旧行为，claude 用自己的默认）。
/// 3. **Direct / 隐式 Direct**：第二个 tuple 元素是 `--model` 用的 model
///    id。来源优先级：
///    - `state.default_selection.model`（如果 default_selection.provider 与
///      本次 spawn 用的 provider 一致 —— 用户在「设为默认（DIRECT）」时已
///      同步 overlay 的 default_model）；
///    - overlay item 的 `default_model`（UI 源；「设为默认」前的 fallback，
///      保证已编辑 default_model 但忘了「设为默认」的用户也不会丢偏好）；
///    - `None`（两者都缺）。
///
/// 失败语义（spec 2026-08-17 §2.2）：一律 `Error { reason }` + warn。绝不
/// panic / 绝不静默回退 `Off` —— 旧 silent Off 让用户看到「claude 启动了
/// 但啥都没发生」时无法定位是 sebas 配置问题还是 claude 自己 env 的问题。
/// 新行为：spawn wrapper 检测 `SEBAS_PROVIDER_ERROR` 后 print + exit(1)。
pub fn compute_provider_resolution(
    state: &ProviderRuntimeState,
    gateway_cfg: Option<&GatewayConfig>,
) -> (ProviderResolution, Option<String>) {
    // 把「Off 但 default_selection.provider 已设」归一为隐式 Direct。
    let effective_mode: ProviderMode = match &state.mode {
        ProviderMode::Off => state
            .default_selection
            .as_ref()
            .map(|d| ProviderMode::Direct {
                provider: d.provider.clone(),
            })
            .unwrap_or(ProviderMode::Off),
        other => other.clone(),
    };
    match &effective_mode {
        ProviderMode::Off => (ProviderResolution::Off, None),
        ProviderMode::Gateway => match gateway_cfg {
            Some(cfg) => gateway_resolution(cfg),
            None => {
                let reason =
                    "ProviderMode::Gateway but no gateway config provided (config.toml missing?)"
                        .to_string();
                tracing::warn!(
                    "ProviderMode::Gateway but no gateway config provided; aborting spawn"
                );
                (ProviderResolution::Error { reason }, None)
            }
        },
        ProviderMode::Direct { provider } => {
            // 优先读 overlay（用户 bot 里改的）；overlay 缺则回退到
            // gateway_cfg 里的同名 provider（仅 config.toml 种子，没经过
            // /provider 编辑的 provider 走这条路径）。
            let (resolution, overlay_model) = if let Some(item) = read_overlay_item(provider) {
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
                // 两者都缺 → 这位 provider 名既不指向 overlay 项、也不指向
                // gateway seed。按持久化层的约定（state_store.rs §虚引用、「必须
                // 存在于 providers 或 gateway_cfg，否则 spawn-time 兜底回退
                // Off + warn」），回退 Off + warn，而不是拒绝启动：这条路径可能
                // 来自泄漏进 state.json 的幽灵 provider（如测试字面量
                // "env-override"），不该让用户连 claude 都拉不起来。真正把
                // provider 名拼错 / 配置残缺的 case，下面 direct_resolution_*
                // 各自的 URL / 密钥校验仍会喷 Error（§2.2 语义保留）。
                tracing::warn!(
                    provider = %provider,
                    "Direct provider not found in overlay or gateway config; falling back to Off"
                );
                (ProviderResolution::Off, None)
            };
            // 第二元素合并：state.default_selection.model（仅在 provider 名
            // 匹配时采用）→ overlay_model → None。
            let model = state
                .default_selection
                .as_ref()
                .filter(|d| d.provider == *provider)
                .and_then(|d| d.model.clone())
                .filter(|s| !s.is_empty())
                .or(overlay_model);
            (resolution, model)
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
        (acp_claude::AgentProtocol::Anthropic, u.to_string())
    } else if let Some(u) = p.base_url_openai.as_deref() {
        (acp_claude::AgentProtocol::OpenAi, u.to_string())
    } else {
        let reason = format!(
            "direct provider '{name}' in gateway config has no base_url_anthropic or base_url_openai"
        );
        tracing::warn!(
            provider = %name,
            "Direct provider missing URLs in gateway config; aborting spawn"
        );
        return (ProviderResolution::Error { reason }, None);
    };
    let auth_token = if let Some(env_var) = &p.api_key_env {
        match std::env::var(env_var) {
            Ok(v) if !v.is_empty() => v,
            _ => {
                let reason =
                    format!("direct provider '{name}' api_key_env '{env_var}' is unset or empty");
                tracing::warn!(
                    provider = %name,
                    env_var = %env_var,
                    "Direct provider api_key_env unset/empty; aborting spawn"
                );
                return (ProviderResolution::Error { reason }, None);
            }
        }
    } else if let Some(plain) = &p.api_key {
        tracing::warn!(
            provider = %name,
            "Direct provider uses plaintext api_key (config.toml-supplied); prefer api_key_env"
        );
        plain.clone()
    } else {
        let reason = format!(
            "direct provider '{name}' has neither api_key_env nor api_key in gateway config"
        );
        tracing::warn!(
            provider = %name,
            "Direct provider has neither api_key_env nor api_key; aborting spawn"
        );
        return (ProviderResolution::Error { reason }, None);
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
/// 设计（spec 2026-08-17 §2.2）：
/// - `extra_env` 来自 driver 的 `resolve_env`（已按 `ProviderMode` 翻译）。
///   `Error` 变体下 driver 已经把 `SEBAS_PROVIDER_ERROR=<reason>` 放进来
///   ——  这是 in-band signal，spawn wrapper（`session_boot`）看到它就 abort。
/// - `extra_args` 来自 driver 的 `resolve_args` ∪ `--model <name>`（仅在
///   `default_model` 非空时附加）。`Error` 变体下两者都空，因为根本没
///   解析出 provider 模型。
/// - 单条 warn log "spawn aborted: provider config error: <reason>"
///   在这里打（不是每个 fallback 分支都打），避免重复 / 漏打。
pub fn resolve_spawn_overrides(
    driver: &ClaudeCodeDriver,
    state: &ProviderRuntimeState,
    gateway_cfg: Option<&GatewayConfig>,
) -> (Vec<(String, String)>, Vec<String>) {
    let (resolution, default_model) = compute_provider_resolution(state, gateway_cfg);
    if let ProviderResolution::Error { reason } = &resolution {
        tracing::warn!(
            reason = %reason,
            "spawn aborted: provider config error: {reason}"
        );
    }
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
    use acp_claude::{AgentProtocol, ClaudeCodeDriver};
    use gateway::config::GatewayConfig;
    use router::provider_state::{ProviderMode, ProviderRuntimeState};
    use router::state_store::DefaultSelection;
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
            default_selection: Some(DefaultSelection::new(name)),
        }
    }

    fn gateway_state() -> ProviderRuntimeState {
        ProviderRuntimeState {
            mode: ProviderMode::Gateway,
            default_selection: None,
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
                assert_eq!(proto, AgentProtocol::Anthropic);
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
                assert_eq!(proto, AgentProtocol::OpenAi);
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
        // state.default_selection 没带 model → 回退到 overlay 的 default_model。
        // spec §2.8 把这一行为锁死：用户在「设为默认（DIRECT）」前编辑
        // default_model 也不会丢偏好。
        let state = direct_state("deepseek");
        let (env, args) = resolve_spawn_overrides(&driver(), &state, None);
        // 验证 --model 出现在 args 末尾（顺序：resolve_args 返回空 + 我们加 --model）。
        assert_eq!(
            args,
            vec!["--model".to_string(), "deepseek-reasoner".to_string()]
        );
        // env 仍包含 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN，证明 driver 也跑了。
        assert!(env.iter().any(|(k, _)| k == "ANTHROPIC_BASE_URL"));
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("DEEPSEEK_API_KEY");
        }
    }

    /// spec §2.8：当 `state.default_selection.model` 显式设置时，spawn 必须
    /// 用它（而不是 overlay 的 `default_model`）生成 `--model <id>`。这是
    /// 「设为默认（DIRECT）」动作的副效：`default_selection.model` 是用户
    /// 当前的偏好，应该凌驾 overlay（catalog）上的任何值。
    #[test]
    fn direct_default_selection_model_overrides_overlay_default_model() {
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
        // state.default_selection.model = "deepseek-chat" 显式覆盖 overlay 的
        // "deepseek-reasoner"。
        let mut state = direct_state("deepseek");
        state.default_selection = Some(DefaultSelection::with_model("deepseek", "deepseek-chat"));
        let (env, args) = resolve_spawn_overrides(&driver(), &state, None);
        assert_eq!(
            args,
            vec!["--model".to_string(), "deepseek-chat".to_string()]
        );
        assert!(env.iter().any(|(k, _)| k == "ANTHROPIC_BASE_URL"));
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("DEEPSEEK_API_KEY");
        }
    }

    /// spec §2.8：Off 模式 + default_selection 已设 → 视为隐式 Direct，
    /// 用 default_selection.provider 解析，spawn 出对应 provider 的 env。
    /// 这是 spec §2.8 新行为：用户没显式切 Direct 但已「设为默认」时也
    /// 应该让默认 provider 生效。
    #[test]
    fn off_with_default_selection_implicit_direct_emits_provider_env() {
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
        let state = ProviderRuntimeState {
            mode: ProviderMode::Off,                                    // 显式 Off
            default_selection: Some(DefaultSelection::new("deepseek")), // 但有默认
        };
        let (resolution, model) = compute_provider_resolution(&state, None);
        match resolution {
            ProviderResolution::Direct {
                proto,
                base_url,
                auth_token,
            } => {
                assert_eq!(proto, AgentProtocol::Anthropic);
                assert_eq!(base_url, "https://api.deepseek.com/anthropic");
                assert_eq!(auth_token, "sk-ds");
            }
            other => panic!("Off+default 应归一为隐式 Direct；got {other:?}"),
        }
        assert!(model.is_none(), "overlay 没设 default_model → model=None");
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("DEEPSEEK_API_KEY");
        }
    }

    /// spec §2.8：Off + default_selection 已设 + model 也设了 → 隐式 Direct
    /// + `--model` 出现在 args。
    #[test]
    fn off_with_default_selection_emits_model_arg_via_implicit_direct() {
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
        let state = ProviderRuntimeState {
            mode: ProviderMode::Off,
            default_selection: Some(DefaultSelection::with_model("deepseek", "deepseek-chat")),
        };
        let (env, args) = resolve_spawn_overrides(&driver(), &state, None);
        assert_eq!(
            args,
            vec!["--model".to_string(), "deepseek-chat".to_string()]
        );
        assert!(env.iter().any(|(k, _)| k == "ANTHROPIC_BASE_URL"));
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("DEEPSEEK_API_KEY");
        }
    }

    /// spec §2.8：default_selection.provider 与 mode.provider 不一致时，
    /// spawn 用 mode.provider（mode 永远胜出），但 model 仍读
    /// default_selection.model（如果存在且 provider 名匹配）。
    /// 现实场景：用户切到 Direct{anthropic} 但 default_selection 还指向
    /// deepseek —— mode 决策优先；但用户给 default_selection 设过的 model
    /// 偏好不会被错绑到 anthropic。
    #[test]
    fn direct_uses_mode_provider_over_default_selection_provider() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "anthropic": {
                        "base_url_anthropic": "https://api.anthropic.com",
                        "api_key_env": "ANTHROPIC_API_KEY"
                    },
                    "deepseek": {
                        "base_url_anthropic": "https://api.deepseek.com/anthropic",
                        "api_key_env": "DEEPSEEK_API_KEY"
                    }
                }
            }"#,
        );
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "sk-anth");
        }
        let state = ProviderRuntimeState {
            mode: ProviderMode::Direct {
                provider: "anthropic".into(),
            },
            default_selection: Some(DefaultSelection::with_model("deepseek", "deepseek-chat")),
        };
        let (resolution, model) = compute_provider_resolution(&state, None);
        match resolution {
            ProviderResolution::Direct { base_url, .. } => {
                assert_eq!(base_url, "https://api.anthropic.com");
            }
            other => panic!("expected Direct(anthropic), got {other:?}"),
        }
        // mode.provider = "anthropic" 与 default_selection.provider = "deepseek"
        // 不一致 → default_selection.model 不被采用（避免「给 anthropic 加
        // deepseek 的 model」这种诡异行为）。overlay 没设 default_model →
        // 兜底 None。
        assert_eq!(
            model, None,
            "provider 名不匹配时 default_selection.model 不用"
        );
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
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
        // 幽灵 / 拼错的 provider：overlay 与 gateway 都没有 → 兜底 Off + warn，
        // 不拒绝启动（持久化层约定的 backoff）。
        assert!(
            matches!(resolution, ProviderResolution::Off),
            "missing provider must fall back to Off, got {resolution:?}"
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
        // 已 tombstone 的 provider 视同「找不到」→ 兜底 Off + warn。
        assert!(
            matches!(resolution, ProviderResolution::Off),
            "tombstoned provider must fall back to Off, got {resolution:?}"
        );
    }

    #[test]
    fn direct_overlay_api_key_env_unset_returns_error() {
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
        match &resolution {
            ProviderResolution::Error { reason } => {
                assert!(
                    reason.contains("deepseek"),
                    "reason must name the provider; got: {reason}"
                );
                assert!(
                    reason.contains("THIS_KEY_IS_NOT_SET_63F8"),
                    "reason must name the env var that is unset; got: {reason}"
                );
            }
            other => panic!("unset api_key_env must yield Error, got {other:?}"),
        }
        // SAFETY: ENV_LOCK held.
        // (no env var to remove since we never set it)
    }

    #[test]
    fn direct_overlay_no_url_returns_error() {
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
        match &resolution {
            ProviderResolution::Error { reason } => {
                assert!(
                    reason.contains("weird"),
                    "reason must name the provider; got: {reason}"
                );
                assert!(
                    reason.contains("no base_url"),
                    "reason must explain which field is missing; got: {reason}"
                );
            }
            other => panic!("missing URLs must yield Error, got {other:?}"),
        }
    }

    #[test]
    fn direct_overlay_missing_api_key_env_returns_error() {
        // 第 5 个 fallback 用例：overlay 配了 base_url 但完全没配密钥
        // （既没 api_key 也没 api_key_env）。旧行为：静默回退 Off；
        // 新行为：返回 Error，spawn wrapper abort。
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "deepseek": {
                        "base_url_anthropic": "https://api.deepseek.com/anthropic"
                    }
                }
            }"#,
        );
        let state = direct_state("deepseek");
        let (resolution, _) = compute_provider_resolution(&state, None);
        match &resolution {
            ProviderResolution::Error { reason } => {
                assert!(
                    reason.contains("deepseek"),
                    "reason must name the provider; got: {reason}"
                );
                assert!(
                    reason.contains("api_key"),
                    "reason must explain missing credential; got: {reason}"
                );
            }
            other => panic!("missing credential must yield Error, got {other:?}"),
        }
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
                assert_eq!(proto, AgentProtocol::Anthropic);
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
    fn gateway_mode_without_cfg_returns_error() {
        let _g = ENV_LOCK.lock().unwrap();
        let state = gateway_state();
        let (resolution, _) = compute_provider_resolution(&state, None);
        match &resolution {
            ProviderResolution::Error { reason } => {
                assert!(
                    reason.contains("Gateway"),
                    "reason must mention Gateway mode; got: {reason}"
                );
                assert!(
                    reason.to_lowercase().contains("no gateway config"),
                    "reason must explain missing config; got: {reason}"
                );
            }
            other => panic!("Gateway mode without cfg must yield Error, got {other:?}"),
        }
    }

    #[test]
    fn gateway_mode_empty_listen_returns_error() {
        let _g = ENV_LOCK.lock().unwrap();
        // parse 走一遍拿到合法 cfg，再把 listen 改成空——这样我们精确覆盖
        // `gateway_mode_uses_http_listen_url_and_first_auth_token` 的反向分支。
        let mut cfg = test_gateway("127.0.0.1:8787", vec!["sk-gw".to_string()]);
        cfg.listen = "".to_string();
        let state = gateway_state();
        let (resolution, _) = compute_provider_resolution(&state, Some(&cfg));
        match &resolution {
            ProviderResolution::Error { reason } => {
                assert!(
                    reason.contains("listen"),
                    "reason must mention listen field; got: {reason}"
                );
            }
            other => panic!("Gateway mode with empty listen must yield Error, got {other:?}"),
        }
    }

    /// Spec 2026-08-17 §2.2：Error 变体经过 driver 翻译后，
    /// `resolve_spawn_overrides` 必须把 `SEBAS_PROVIDER_ERROR=<reason>`
    /// 放进 `extra_env`，spawn wrapper 据此 abort + exit(1)。
    #[test]
    fn resolve_spawn_overrides_error_emits_sebas_provider_error_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_overlay_env();
        // 走 gateway_mode_without_cfg 触发 Error。
        let state = gateway_state();
        let (env, args) = resolve_spawn_overrides(&driver(), &state, None);
        assert!(
            env.iter().any(|(k, _)| k == "SEBAS_PROVIDER_ERROR"),
            "Error variant must inject SEBAS_PROVIDER_ERROR; got env = {env:?}"
        );
        // 没解析出 provider → 不应有 --model / 其他 args。
        assert!(
            args.is_empty(),
            "Error variant must not emit any args; got {args:?}"
        );
        // 也不应有 provider-shaped env（不能给 agent 看 partial state）。
        assert!(
            !env.iter().any(|(k, _)| k.starts_with("ANTHROPIC_")),
            "Error variant must not leak ANTHROPIC_* env; got {env:?}"
        );
        assert!(
            !env.iter().any(|(k, _)| k.starts_with("OPENAI_")),
            "Error variant must not leak OPENAI_* env; got {env:?}"
        );
        // env 里只有 SEBAS_PROVIDER_ERROR 一条。
        assert_eq!(
            env.len(),
            1,
            "Error variant env must contain only the signal"
        );
    }

    #[test]
    fn gateway_mode_emits_anthropic_env_via_driver() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = test_gateway("127.0.0.1:8787", vec!["sk-gw".to_string()]);
        let state = gateway_state();
        let (env, args) = resolve_spawn_overrides(&driver(), &state, Some(&cfg));
        assert!(
            env.iter()
                .any(|(k, v)| k == "ANTHROPIC_BASE_URL" && v == "http://127.0.0.1:8787")
        );
        assert!(
            env.iter()
                .any(|(k, v)| k == "ANTHROPIC_AUTH_TOKEN" && v == "sk-gw")
        );
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
            r#"{"version":2,"providers":{"test_prov":{"preset":"deepseek","base_url_anthropic":"https://example.test/anthropic","api_key":"sk-test-direct"}},"deleted":[],"mode":{"kind":"off"},"default_selection":null}"#,
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
            r#"{"version":2,"providers":{"test_prov":{"preset":"deepseek","base_url_anthropic":"https://example.test/anthropic","api_key":"sk-test-direct"}},"deleted":[],"mode":{"kind":"direct","provider":"test_prov"},"default_selection":{"provider":"test_prov"}}"#,
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
                assert_eq!(proto, AgentProtocol::Anthropic);
                assert_eq!(base_url, "https://example.test/anthropic");
                assert_eq!(auth_token, "sk-test-direct");
            }
            other => panic!("expected Direct, got {other:?}"),
        }
        // driver 必须把 Direct 翻译成 ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN，
        // 并把这两个变量送给 subprocess。args 空因为 overlay 里没设 default_model。
        assert!(
            env.iter()
                .any(|(k, v)| k == "ANTHROPIC_BASE_URL" && v == "https://example.test/anthropic")
        );
        assert!(
            env.iter()
                .any(|(k, v)| k == "ANTHROPIC_AUTH_TOKEN" && v == "sk-test-direct")
        );
        assert!(args.is_empty(), "no default_model → no --model args");

        // --- Scenario C: Gateway → Gateway ---
        std::fs::write(
            &state_path,
            r#"{"mode":{"kind":"gateway"},"default_selection":null}"#,
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

        // --- Scenario D: Direct + 不存在的 provider → 兜底 Off（spec 2026-08-17
        //     §2.6 持久化层约定「找不到就回退 Off + warn」）。此前这里返回
        //     `ProviderResolution::Error` → spawn wrapper `exit(1)`，用户因一个
        //     幽灵 provider 名（如泄漏的测试字面量 env-override）连 claude 都
        //     拉不起来；改成回退 Off 后启动不被阻断。---
        std::fs::write(
            &state_path,
            r#"{"mode":{"kind":"direct","provider":"nonexistent"},"default_selection":null}"#,
        )
        .unwrap();
        let st = router::provider_state::load();
        let (env, args) = resolve_spawn_overrides(&driver(), &st, None);
        match compute_provider_resolution(&st, None).0 {
            ProviderResolution::Off => {}
            other => {
                panic!(
                    "missing Direct provider 必须兜底回退 Off（不再 Error/abort）；got {other:?}"
                )
            }
        }
        // 兜底 Off：env / args 都空（driver 不发 provider env），且绝无
        // SEBAS_PROVIDER_ERROR —— 否则 spawn wrapper 仍会拒绝启动。
        assert!(
            env.is_empty(),
            "兜底 Off 不应给 driver 任何 env；got {env:?}"
        );
        assert!(args.is_empty(), "兜底 Off 不应有任何 args；got {args:?}");
        assert!(
            !env.iter().any(|(k, _)| k == "SEBAS_PROVIDER_ERROR"),
            "兜底 Off 不应注入 SEBAS_PROVIDER_ERROR（否则仍会 abort）；got {env:?}"
        );

        // 清理 env var，避免污染后续测试 / CI 环境。
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("SEBAS_STATE_FILE");
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
        }
    }

    /// Direct provider 的 overlay item 同时填了 anthropic + openai base_url
    /// 且**未显式指定 `protocol` 字段**时，走「auto」默认（spec 2026-08-17
    /// §2.4）：优先 Anthropic 协议面。该测试锁定 auto 默认值，避免日后
    /// 被偷改。显式 `protocol=openai` 走 OpenAI 由另一个测试覆盖。
    #[tokio::test]
    async fn direct_prefers_anthropic_when_both_base_urls_set() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
              "providers": {
                "dual": {
                  "name": "dual",
                  "base_url_anthropic": "https://example.com/anthropic",
                  "base_url_openai": "https://example.com/openai",
                  "api_key": "sk-test"
                }
              }
            }"#,
        );
        unsafe {
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                dir.path().join("providers.json").to_str().unwrap(),
            );
        }
        let state = direct_state("dual");
        let (resolution, _) = compute_provider_resolution(&state, None);
        match resolution {
            ProviderResolution::Direct {
                proto, base_url, ..
            } => {
                assert_eq!(
                    proto,
                    AgentProtocol::Anthropic,
                    "anthropic 协议优先于 openai"
                );
                assert_eq!(base_url, "https://example.com/anthropic");
            }
            other => panic!("expected Direct, got {other:?}"),
        }
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
        }
    }

    /// spec 2026-08-17 §2.4：overlay 里 `protocol=openai` 显式声明 +
    /// 两个 URL 都配了 → 强制走 OpenAI（不再走 auto 的 anthropic 优先）。
    #[tokio::test]
    async fn direct_explicit_protocol_openai_with_both_urls_uses_openai() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
              "providers": {
                "dual": {
                  "name": "dual",
                  "base_url_anthropic": "https://example.com/anthropic",
                  "base_url_openai": "https://example.com/openai",
                  "api_key": "sk-test",
                  "protocol": "openai"
                }
              }
            }"#,
        );
        unsafe {
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                dir.path().join("providers.json").to_str().unwrap(),
            );
        }
        let state = direct_state("dual");
        let (resolution, _) = compute_provider_resolution(&state, None);
        match resolution {
            ProviderResolution::Direct {
                proto, base_url, ..
            } => {
                assert_eq!(
                    proto,
                    AgentProtocol::OpenAi,
                    "显式 protocol=openai 必须强制 OpenAI"
                );
                assert_eq!(base_url, "https://example.com/openai");
            }
            other => panic!("expected Direct, got {other:?}"),
        }
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
        }
    }

    /// spec 2026-08-17 §2.4 + §2.2：overlay 里 `protocol=anthropic` 但只配了
    /// openai URL → 显式选择必须能命中；命中失败时不再静默回退 Off，
    /// 而是返回 Error（spawn wrapper abort + 用户看到错误）。
    #[tokio::test]
    async fn direct_explicit_protocol_anthropic_with_only_openai_url_returns_error() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
              "providers": {
                "oai-only": {
                  "name": "oai-only",
                  "base_url_openai": "https://example.com/openai",
                  "api_key": "sk-test",
                  "protocol": "anthropic"
                }
              }
            }"#,
        );
        unsafe {
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                dir.path().join("providers.json").to_str().unwrap(),
            );
        }
        let state = direct_state("oai-only");
        let (resolution, _) = compute_provider_resolution(&state, None);
        match &resolution {
            ProviderResolution::Error { reason } => {
                assert!(
                    reason.contains("oai-only"),
                    "reason must name the provider; got: {reason}"
                );
                assert!(
                    reason.contains("anthropic"),
                    "reason must mention the protocol; got: {reason}"
                );
                assert!(
                    reason.contains("base_url_anthropic"),
                    "reason must explain which URL field is missing; got: {reason}"
                );
            }
            other => panic!(
                "显式 protocol=anthropic 缺 base_url_anthropic → 必须 Error，不能 fallback 到 OpenAI；got {other:?}"
            ),
        }
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
        }
    }

    /// spec 2026-08-17 §2.4 + §2.2：overlay 里 `protocol=openai` 但只配了
    /// anthropic URL → 同样返回 Error（对称分支）。
    #[tokio::test]
    async fn direct_explicit_protocol_openai_with_only_anthropic_url_returns_error() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
              "providers": {
                "anth-only": {
                  "name": "anth-only",
                  "base_url_anthropic": "https://example.com/anthropic",
                  "api_key": "sk-test",
                  "protocol": "openai"
                }
              }
            }"#,
        );
        unsafe {
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                dir.path().join("providers.json").to_str().unwrap(),
            );
        }
        let state = direct_state("anth-only");
        let (resolution, _) = compute_provider_resolution(&state, None);
        match &resolution {
            ProviderResolution::Error { reason } => {
                assert!(
                    reason.contains("anth-only"),
                    "reason must name the provider; got: {reason}"
                );
                assert!(
                    reason.contains("openai"),
                    "reason must mention the protocol; got: {reason}"
                );
                assert!(
                    reason.contains("base_url_openai"),
                    "reason must explain which URL field is missing; got: {reason}"
                );
            }
            other => panic!("显式 protocol=openai 缺 base_url_openai → 必须 Error；got {other:?}"),
        }
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
        }
    }

    /// spec 2026-08-17 §2.4：overlay 里 `protocol=anthropic` 显式 +
    /// 两个 URL 都配了 → 强制走 Anthropic（覆盖 auto 优先级）。
    #[tokio::test]
    async fn direct_explicit_protocol_anthropic_with_both_urls_uses_anthropic() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
              "providers": {
                "dual": {
                  "name": "dual",
                  "base_url_anthropic": "https://example.com/anthropic",
                  "base_url_openai": "https://example.com/openai",
                  "api_key": "sk-test",
                  "protocol": "anthropic"
                }
              }
            }"#,
        );
        unsafe {
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                dir.path().join("providers.json").to_str().unwrap(),
            );
        }
        let state = direct_state("dual");
        let (resolution, _) = compute_provider_resolution(&state, None);
        match resolution {
            ProviderResolution::Direct {
                proto, base_url, ..
            } => {
                assert_eq!(proto, AgentProtocol::Anthropic);
                assert_eq!(base_url, "https://example.com/anthropic");
            }
            other => panic!("expected Direct, got {other:?}"),
        }
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
        }
    }

    /// Gateway 模式 + gateway config 里有 listen 但 auth_token 是空数组：
    /// 不应 panic / 不应拒绝；URL 仍构造，auth_token 是空字符串（agent 会
    /// 在没 Bearer 的情况下调 gateway，gateway 自己拒）。这是用户故意不配
    /// auth 的合法状态。
    #[tokio::test]
    async fn gateway_with_empty_auth_token_still_constructs_url() {
        let raw = r#"
[gateway]
listen = "127.0.0.1:8787"
auth_token = []
[provider.anthropic]
base_url_anthropic = "https://api.anthropic.com"
"#;
        let cfg = GatewayConfig::parse(raw).expect("test gateway parses");
        let state = ProviderRuntimeState {
            mode: ProviderMode::Gateway,
            default_selection: None,
        };
        let (resolution, _) = compute_provider_resolution(&state, Some(&cfg));
        match resolution {
            ProviderResolution::Gateway { url, auth_token } => {
                assert_eq!(url, "http://127.0.0.1:8787");
                assert_eq!(
                    auth_token, "",
                    "空 auth_token 数组 → 空字符串（agent 调 gateway 不带 Bearer）"
                );
            }
            other => panic!("expected Gateway, got {other:?}"),
        }
    }
}
