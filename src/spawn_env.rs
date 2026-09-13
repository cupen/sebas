//! Spawn-time env/args translation from `ProviderMode` + `DefaultSelection`.
//!
//! Sits between two already-built pieces:
//!
//! - `sebas_dispatch::provider_state::ProviderRuntimeState`（bead sebas-63f.3）：runtime 决策
//!   ——「当前走 Off / Direct / Router」。
//! - `sebas_acp::claude::ClaudeCodeDriver`（bead sebas-63f.2）：把 `ProviderResolution`
//!   翻成 agent 进程看得懂的 env vars + CLI args。
//!
//! 这里只负责「中间环节」：从 state 拿到语义意图 → 解析上游 URL + 密钥 → 喂
//! 给 driver。对调用方（`acp_spawn_and_activate` / `acp_resume_and_activate`）
//! 暴露一个统一的 [`resolve_spawn_overrides`]，返回 `(extra_env, extra_args)`，
//! 追加到 `claude_args` 上送进 `SessionManager::create_session`。
//!
//! 失败语义（openspec/specs/provider-management/spec.md）：spawn-time 解析失败（router URL 没配、
//! named provider 在 overlay 里找不到、api_key_env 没值）一律返回
//! `ProviderResolution::Error { reason }`。driver 把这个变体翻译成单条
//! `SEBAS_PROVIDER_ERROR=<reason>` env var，spawn wrapper（`session_boot`）
//! 看到这条 var 就立刻 `print` + `exit(1)`，不真的去 fork claude 子进程。
//! 旧行为是回退到 `ProviderResolution::Off`，让 claude 用自己 env / config
//! —— 用户看到"启动了但啥都没发生"时无法定位是 sebas 的问题还是 claude
//! 自己环境的问题。新行为把错误直接喂给用户。

use sebas_acp::claude::{ClaudeCodeDriver, ProviderResolution};
use sebas_router::config::RouterConfig;
use sebas_dispatch::provider_state::{ProviderMode, ProviderRuntimeState};
use serde_json::{Map, Value};

/// Claude Code 模型 env 覆盖集的 4 个 `ANTHROPIC_MODEL` 键 + 1 个
/// `CLAUDE_CODE_SUBAGENT_MODEL` 键（openspec/changes/acp-claude-model-env-cover）。
///
/// 档位语义：provider 的 `models`（强→弱）经 `sebas_router::models::map_to_env`
/// 映射成 OPUS/SONNET/HAIKU 三档；`CLAUDE_CODE_SUBAGENT_MODEL` 回退到最弱档
/// （= HAIKU 值）。provider 未配 models → 不强制覆盖，claude 用自己发现。
const CLAUDE_SUBAGENT_MODEL_ENV: &str = "CLAUDE_CODE_SUBAGENT_MODEL";


/// 读单个 provider 的原始 Item（含 `default_model`）。make-core-own-provider-data
/// 4.1：状态库是权威——`state_store::load()` 优先走 core 的 state store
/// engine，**不再优先读 legacy `providers.json`**（旧文件会盖住库里的
/// `default_model`，是 live correctness bug）。
///
/// 4.2 降级路径：state store 未初始化（engine 缺失，如 DB 初始化失败/测试
/// 夹具）时 `state_store::load()` 自行回退读 state.json / providers.json
/// 文件——该路径保留并如实上报来源（warn 一行，注明数据来自 legacy 文件
/// 而非状态库）。文件不存在 / JSON 坏 / 名字不在 overrides 里 / 已
/// tombstone → `None`（不报错，让上层决定 graceful fallback 到 `Off`）。
///
/// `default_model` 只在条目上（router `ProviderConfig` 没有这字段，故意不向
/// router 同步 —— sebas-63f.4 设计决定），所以必须从这里读，不能从
/// `router_cfg.providers` 拿。
fn read_overlay_item(name: &str) -> Option<Map<String, Value>> {
    let store_live = sebas_dispatch::state_store::engine().is_some();
    let state = sebas_dispatch::state_store::load();
    if !store_live {
        // 4.2：无状态库时的文件降级读取——如实上报来源，不冒充权威。
        tracing::warn!(
            provider = %name,
            "state store 未初始化：provider 数据来自 legacy 文件降级读取（state.json / providers.json），非状态库权威"
        );
    }
    if state.deleted.iter().any(|d| d == name) {
        return None;
    }
    state.providers.get(name).cloned()
}

/// 把 overlay Item 映射到 `ProviderResolution::Direct`。
///
/// URL 来源：自定义 provider 直接读条目上的 `base_url_anthropic` /
/// `base_url_openai_chat`；preset 派生条目不落盘 url —— 从代码内置 preset
/// 表物化（跟随代码，openspec/specs/provider-management/spec.md）。
/// Responses 槽位不参与 Direct spawn——agent 不说 Responses 协议，
/// responses-only provider 显式报错。
///
/// 协议选择（UI 在 `/provider` 详情面板里暴露的「协议」radio 写到这里）：
/// - `"anthropic"` → 强制走 anthropic 槽；缺失 → `Error` + warn。
/// - `"openai"`    → 强制走 chat 槽；缺失 → `Error` + warn。
/// - `"auto"` / 缺省 → 优先 anthropic 槽，其次 chat 槽。两者都缺 → `Error` + warn。
///
/// 密钥优先级：`api_key` 明文（debug 提示）> `api_key_env` 读 env（preset
/// 派生条目缺省继承 preset 的默认 env 名）；env 缺失/空 → `Error` + warn。
/// 和 `RouterConfig::resolve_api_keys` 走同一套优先级，行为一致。
fn direct_resolution_from_overlay(
    name: &str,
    item: &Map<String, Value>,
) -> (ProviderResolution, Option<String>) {
    // preset 物化源：条目带 `preset` 字段时从代码表取连接数据。
    let preset = item
        .get("preset")
        .and_then(Value::as_str)
        .and_then(|pn| {
            sebas_router::config::presets()
                .iter()
                .find(|p| p.name == pn)
        });
    let item_url = |key: &str| -> Option<String> {
        item.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let base_url_anthropic = item_url("base_url_anthropic")
        .or_else(|| preset.and_then(|p| p.base_url_anthropic).map(str::to_string));
    let base_url_openai_chat = item_url("base_url_openai_chat")
        .or_else(|| preset.and_then(|p| p.base_url_openai_chat).map(str::to_string));
    let default_model = item_url("default_model");
    // 协议选择：UI 在详情面板的 radio。缺省 = "auto" = anthropic 优先。
    let protocol = item
        .get("protocol")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("auto");

    let (proto, base_url) = match protocol {
        "anthropic" => match base_url_anthropic {
            Some(u) => (sebas_acp::claude::AgentProtocol::Anthropic, u),
            None => {
                let reason = format!(
                    "direct provider '{name}' has explicit protocol=anthropic but no base_url_anthropic"
                );
                tracing::warn!(
                    provider = %name,
                    "direct provider sets protocol=anthropic but base_url_anthropic is missing, aborting spawn"
                );
                return (ProviderResolution::Error { reason }, default_model);
            }
        },
        "openai" => match base_url_openai_chat {
            Some(u) => (sebas_acp::claude::AgentProtocol::OpenAi, u),
            None => {
                let reason = format!(
                    "direct provider '{name}' has explicit protocol=openai but no base_url_openai_chat (a responses-only provider is not spawnable in Direct mode)"
                );
                tracing::warn!(
                    provider = %name,
                    "direct provider sets protocol=openai but base_url_openai_chat is missing, aborting spawn"
                );
                return (ProviderResolution::Error { reason }, default_model);
            }
        },
        // "auto" 或未知值：anthropic > openai chat。
        _ => {
            if let Some(u) = base_url_anthropic {
                (sebas_acp::claude::AgentProtocol::Anthropic, u)
            } else if let Some(u) = base_url_openai_chat {
                (sebas_acp::claude::AgentProtocol::OpenAi, u)
            } else {
                let reason = format!(
                    "direct provider '{name}' has no base_url_anthropic or base_url_openai_chat"
                );
                tracing::warn!(
                    provider = %name,
                    "Direct provider has no base_url_anthropic / base_url_openai_chat; aborting spawn"
                );
                return (ProviderResolution::Error { reason }, default_model);
            }
        }
    };

    // 密钥：api_key 明文优先，否则读 api_key_env（preset 派生缺省继承
    // preset 的默认 env 名）；都没有 → Error。
    let preset_env = preset.map(|p| p.api_key_env);
    let auth_token = if let Some(key) = item
        .get("api_key")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        tracing::debug!(
            provider = %name,
            "Direct provider uses plaintext api_key (overlay-supplied); prefer api_key_env"
        );
        key.to_string()
    } else if let Some(env_var) = item
        .get("api_key_env")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or(preset_env)
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

/// 从 `RouterConfig` 派生 `ProviderResolution::Router`。
///
/// URL 取 `router_cfg.listen`（如 `127.0.0.1:8787`），前面补 `http://`。
/// Auth token 取 `auth_token[0]`（空数组或空字符串 → 不带 auth，但 warn）。
/// 至少 `listen` 必须非空 —— 否则 `Off` + warn。
fn router_resolution(cfg: &RouterConfig) -> (ProviderResolution, Option<String>) {
    let listen = cfg.listen.trim();
    if listen.is_empty() {
        let reason = "ProviderMode::Router but router.listen is empty in config".to_string();
        tracing::warn!("ProviderMode::Router but router.listen is empty; aborting spawn");
        return (ProviderResolution::Error { reason }, None);
    }
    let url = format!("http://{listen}");
    let auth_token = cfg.auth_token.first().cloned().unwrap_or_default();
    if auth_token.is_empty() {
        tracing::warn!(
            listen = %listen,
            "ProviderMode::Router but router.auth_token is empty; agent will call without Bearer/x-api-key"
        );
    }
    (ProviderResolution::Router { url, auth_token }, None)
}

/// 解析 `ProviderMode` + `DefaultSelection` → `ProviderResolution` + 可选的
/// `default_model`（spawn 时追加 `--model <id>`，仅 Direct / Off-with-default
/// 模式下生效；Router 一律 `None`）。
///
/// openspec/specs/provider-management/spec.md 三处决策点：
///
/// 1. **Off + default_selection 已设** → 视为隐式 Direct，按
///    `default_selection.provider` 解析（同 Direct{...} 路径）。这是
///    openspec/specs/provider-management/spec.md 的新行为：用户没显式切 Direct 但已经「设为默认（DIRECT）」时
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
/// 失败语义（openspec/specs/provider-management/spec.md）：一律 `Error { reason }` + warn。绝不
/// panic / 绝不静默回退 `Off` —— 旧 silent Off 让用户看到「claude 启动了
/// 但啥都没发生」时无法定位是 sebas 配置问题还是 claude 自己 env 的问题。
/// 新行为：spawn wrapper 检测 `SEBAS_PROVIDER_ERROR` 后 print + exit(1)。
pub fn compute_provider_resolution(
    state: &ProviderRuntimeState,
    router_cfg: Option<&RouterConfig>,
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
        ProviderMode::Router => match router_cfg {
            Some(cfg) => router_resolution(cfg),
            // 没有可用的 router（config.toml 没有 [router] 段）→ 回退 Off +
            // 段）→ 回退 Off + warn，而不是拒绝启动。mode=router 但 router
            // 没起来时，让 claude 走自己 env 配置（Off 的语义）比整 bot 卡死
            // 在「refusing to launch」强。真把 router 配坏了（listen 空）仍
            // 由 router_resolution 返回 Error（显式失败语义保留）。
            None => {
                tracing::warn!(
                    "ProviderMode::Router but no router config provided; falling back to Off"
                );
                (ProviderResolution::Off, None)
            }
        },
        ProviderMode::Direct { provider } => {
            // 优先读 overlay（用户 bot 里改的）；overlay 缺则回退到
            // router_cfg 里的同名 provider（仅 config.toml 种子，没经过
            // /provider 编辑的 provider 走这条路径）。
            let (resolution, overlay_model) = if let Some(item) = read_overlay_item(provider) {
                direct_resolution_from_overlay(provider, &item)
            } else if let Some(cfg) = router_cfg
                && let Some(p) = cfg.providers.get(provider)
            {
                // router 侧 resolution：已经过 preset 解析与 overlay 合并。
                // 这里用 router 自己的 ProviderConfig 反推 Direct 给 agent：
                //   - base_url_anthropic/openai 决定协议；
                //   - api_key_env 读 env，api_key 明文兜底。
                build_direct_from_router_config(provider, p)
            } else {
                // 两者都缺 → 这位 provider 名既不指向 overlay 项、也不指向
                // router seed。按持久化层的约定（state_store.rs 的契约：「必须
                // 存在于 providers 或 router_cfg，否则 spawn-time 兜底回退
                // Off + warn」），回退 Off + warn，而不是拒绝启动：这条路径可能
                // 来自泄漏进 state.json 的幽灵 provider（如测试字面量
                // "env-override"），不该让用户连 claude 都拉不起来。真正把
                // provider 名拼错 / 配置残缺的 case，下面 direct_resolution_*
                // 各自的 URL / 密钥校验仍会喷 Error（显式失败语义保留）。
                tracing::warn!(
                    provider = %provider,
                    "Direct provider not found in overlay or router config; falling back to Off"
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

/// 从 `sebas_router::config::ProviderConfig` 构造 `ProviderResolution::Direct`。
/// 与 `direct_resolution_from_overlay` 语义一致，只是输入形状不同 —— 复用
/// 同一套协议选择 + 密钥优先级，避免两套逻辑漂移。
fn build_direct_from_router_config(
    name: &str,
    p: &sebas_router::config::ProviderConfig,
) -> (ProviderResolution, Option<String>) {
    // anthropic 槽优先，其次 chat 槽；responses-only provider 不可 spawn
    // （agent 不说 Responses 协议）。
    let (proto, base_url) = if let Some(u) = p.base_url_anthropic.as_deref() {
        (sebas_acp::claude::AgentProtocol::Anthropic, u.to_string())
    } else if let Some(u) = p.base_url_openai_chat.as_deref() {
        (sebas_acp::claude::AgentProtocol::OpenAi, u.to_string())
    } else {
        let reason = format!(
            "direct provider '{name}' in router config has no base_url_anthropic or base_url_openai_chat (a responses-only provider is not spawnable in Direct mode)"
        );
        tracing::warn!(
            provider = %name,
            "Direct provider missing spawnable URLs in router config; aborting spawn"
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
        tracing::debug!(
            provider = %name,
            "Direct provider uses plaintext api_key (config.toml-supplied); prefer api_key_env"
        );
        plain.clone()
    } else {
        let reason = format!(
            "direct provider '{name}' has neither api_key_env nor api_key in router config"
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
        None, // router ProviderConfig 不带 default_model（设计如此）
    )
}

/// 给 claude code 子进程的模型 cover env（openspec/changes/acp-claude-model-env-cover）。
///
/// 由 provider 的 `models` 条目（强→弱）经 `sebas_router::models::map_to_env`
/// 导出 4 个 `ANTHROPIC_MODEL`/`ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`
/// 值，再加 `CLAUDE_CODE_SUBAGENT_MODEL = 最弱档`（= HAIKU 值），总共 5 个键。
/// `models` 为空 → 返回空 Vec，不强制覆盖（模式 Router 或裸 Off 也据此
/// 不盖模型 env，语义见 spec 的 "Transparency across provider modes"）。
/// env 映射按条目 id 解析——能力标记是纯元数据，不影响赋值（task 1.3）。
///
/// 覆盖语义：SDK 最终用 `Command::envs` 增量合并，同键时 extra_env 里的值
/// 会压掉父进程残留（`claude/driver.rs:166-167,213` + cc-agent-sdk
/// `build_env`），符合 spec 的 "Override beats inherited values"。
fn model_cover_env(models: &[sebas_router::models::ModelEntry]) -> Vec<(String, String)> {
    if models.is_empty() {
        return Vec::new();
    }
    let env = sebas_router::models::map_to_env(models);
    let mut out: Vec<(String, String)> = env
        .to_env_map()
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    // SUBAGENT 档位目前回退到最弱模型（haiku 值）；T0/T1/T2 标注落地后
    // 这里换成"按能力标注"的查询。
    if let Some(haiku) = env.haiku {
        out.push((CLAUDE_SUBAGENT_MODEL_ENV.to_string(), haiku));
    }
    out
}

/// 取本 spawn 用到的 provider 的模型列表（预设物化后的强→弱 `models`）。
///
/// `provider` 是调用方（`resolve_spawn_overrides`）依 mode/default_selection
/// 折叠后的 `Direct` provider 名。与 `compute_provider_resolution` 读取同一处
/// overlay / router_cfg，保持 cover 值和端点 env 的来源一致。
///
/// `None` 情形：模式非 Direct（Router/Off 空默认）或 `Direct`/`Off` 但无
/// `default_selection`/`mode.provider`（precise: state 缺 provider）→不盖。
fn effective_provider_models(
    state: &ProviderRuntimeState,
    router_cfg: Option<&RouterConfig>,
) -> Option<Vec<sebas_router::models::ModelEntry>> {
    use sebas_router::models::ModelEntry;
    // 与 compute_provider_resolution 的「Off + default_selection → 隐式
    // Direct」折叠保持同一选择；无 provider 则不盖。
    let provider = match &state.mode {
        ProviderMode::Direct { provider } => provider.clone(),
        ProviderMode::Off => state
            .default_selection
            .as_ref()
            .map(|d| d.provider.clone())?,
        ProviderMode::Router => return None,
    };
    // 用户通过 bot / `/provider` 或 WebUI 编辑的条目带 `models`（custom，
    // 条目对象或遗留裸字符串）或 `preset` 名（preset 派生缺省物化代码表）；
    // overlay 缺项时回退到 router config seed 的 `ProviderConfig.models`。
    // 两者都没有 → 不盖。
    if let Some(item) = read_overlay_item(&provider) {
        let preset_models: Option<Vec<ModelEntry>> = item
            .get("preset")
            .and_then(Value::as_str)
            .and_then(|pn| {
                sebas_router::config::presets()
                    .iter()
                    .find(|p| p.name == pn)
                    .map(|p| p.models.iter().map(|m| m.to_entry()).collect())
            });
        // 条目对象（现行为）与裸字符串（遗留形态，task 1.1 兼容读取）都
        // 接受；单个元素非法（含未知能力标记，task 1.4 拒绝语义）→ 整段
        // 按无 models 处理，回落 preset 代码表 / router seed，绝不半渲染。
        let custom_models: Vec<ModelEntry> = match item.get("models") {
            Some(Value::Array(arr)) if !arr.is_empty() => arr
                .iter()
                .map(|el| serde_json::from_value::<ModelEntry>(el.clone()))
                .collect::<Result<Vec<_>, _>>()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if !custom_models.is_empty() {
            return Some(custom_models);
        }
        if let Some(models) = preset_models.filter(|m| !m.is_empty()) {
            return Some(models);
        }
    }
    router_cfg
        .and_then(|cfg| cfg.providers.get(&provider))
        .filter(|p| !p.models.is_empty())
        .map(|p| p.models.clone())
}

/// 给 agent 进程的额外 env vars + 额外 CLI args。
///
/// 设计（openspec/specs/provider-management/spec.md + acp-claude-model-env-cover）：
/// - `extra_env` 来自 driver 的 `resolve_env`（已按 `ProviderMode` 翻译），
///   再追加 [`model_cover_env`] 生成的 5 个模型键（Direct/Off-with-default 时
///   生效；Router 与裸 Off 不强制覆盖）。`Error` 变体下 driver 已经把
///   `SEBAS_PROVIDER_ERROR=<reason>` 放进来——这是 in-band signal，spawn wrapper
///   看到就 abort。
/// - `extra_args` 来自 driver 的 `resolve_args` ∪ `--model <name>`（仅在
///   `default_model` 非空时附加）。`Error` 变体下两者都空，因为根本没
///   解析出 provider 模型。
/// - 单条 warn log "spawn aborted: provider config error: <reason>"
///   在这里打（不是每个 fallback 分支都打），避免重复 / 漏打。
pub fn resolve_spawn_overrides(
    driver: &ClaudeCodeDriver,
    state: &ProviderRuntimeState,
    router_cfg: Option<&RouterConfig>,
) -> (Vec<(String, String)>, Vec<String>) {
    let (resolution, default_model) = compute_provider_resolution(state, router_cfg);
    if let ProviderResolution::Error { reason } = &resolution {
        tracing::warn!(
            reason = %reason,
            "spawn aborted: provider config error: {reason}"
        );
    }
    let mut env = driver.resolve_env(&resolution);
    if let Some(models) = effective_provider_models(state, router_cfg) {
        env.extend(model_cover_env(&models));
    }
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
    use sebas_acp::claude::{AgentProtocol, ClaudeCodeDriver};
    use sebas_router::config::RouterConfig;
    use sebas_dispatch::provider_state::{ProviderMode, ProviderRuntimeState};
    use sebas_dispatch::state_store::DefaultSelection;
    use std::sync::Mutex;

    // 串行化所有 env 访问：`SEBAS_ROUTER_PROVIDER_OVERLAY` 是全局变量，
    // 跨测试并发跑会撞；与 `router/src/config.rs::tests` 同惯例。
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

    fn router_state() -> ProviderRuntimeState {
        ProviderRuntimeState {
            mode: ProviderMode::Router,
            default_selection: None,
        }
    }

    /// Build a minimal `RouterConfig` for tests — RouterConfig has no
    /// `Default` impl (out of scope for this task), so we set the fields
    /// the spawn-env resolver actually touches (`listen`, `auth_token`,
    /// `providers`) and leave the rest at their defaults via `parse`.
    fn test_router(listen: &str, auth_token: Vec<String>) -> RouterConfig {
        let raw = format!(
            r#"
[router]
listen = "{listen}"
auth_token = {auth_token:?}
# 隔离：不合并开发机 ~/.sebas/providers.json（其 openai 条目与 preset
# 校验冲突导致 parse 失败）。
provider_overlay = "__sebas_spawn_env_no_overlay__.json"
[provider.anthropic]
"#
        );
        RouterConfig::parse(&raw).expect("test router config parses")
    }

    fn write_overlay(dir: &std::path::Path, body: &str) {
        let path = dir.join("providers.json");
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(&path, body).unwrap();
        // SAFETY: ENV_LOCK held across all overlay-touching tests.
        unsafe {
            std::env::set_var("SEBAS_ROUTER_PROVIDER_OVERLAY", path.to_str().unwrap());
        }
    }

    fn clear_overlay_env() {
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("SEBAS_ROUTER_PROVIDER_OVERLAY");
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
                        "base_url_openai_chat": "https://dashscope.aliyuncs.com/compatible-mode/v1",
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
        // openspec/specs/provider-management/spec.md 把这一行为锁死：用户在「设为默认（DIRECT）」前编辑
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

    /// openspec/specs/provider-management/spec.md：当 `state.default_selection.model` 显式设置时，spawn 必须
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

    /// openspec/specs/provider-management/spec.md：Off 模式 + default_selection 已设 → 视为隐式 Direct，
    /// 用 default_selection.provider 解析，spawn 出对应 provider 的 env。
    /// 这是 openspec/specs/provider-management/spec.md 新行为：用户没显式切 Direct 但已「设为默认」时也
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

    /// openspec/specs/provider-management/spec.md：Off + default_selection 已设 + model 也设了 → 隐式 Direct
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

    /// openspec/specs/provider-management/spec.md：default_selection.provider 与 mode.provider 不一致时，
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
        // 幽灵 / 拼错的 provider：overlay 与 router 都没有 → 兜底 Off + warn，
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

    // ---- Direct: router_cfg fallback (no overlay entry) ----

    #[test]
    fn direct_falls_back_to_router_cfg_when_overlay_missing() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(dir.path(), r#"{ "providers": {} }"#);
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "sk-anth-gw");
        }
        // 显式构造一份带 `anthropic` provider 的 router config，让
        // 「overlay 没找到 → router_cfg 兜底」分支被命中。
        let raw = r#"
[router]
listen = "127.0.0.1:8787"
auth_token = "x"
[provider.anthropic]
api_key_env = "ANTHROPIC_API_KEY"
"#;
        let cfg = RouterConfig::parse(raw).expect("test router parses");
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
            other => panic!("expected Direct via router_cfg, got {other:?}"),
        }
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
        }
    }

    // ---- Router ----

    #[test]
    fn router_mode_uses_http_listen_url_and_first_auth_token() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = test_router("127.0.0.1:8787", vec!["sk-gw".to_string()]);
        let state = router_state();
        let (resolution, model) = compute_provider_resolution(&state, Some(&cfg));
        match resolution {
            ProviderResolution::Router { url, auth_token } => {
                assert_eq!(url, "http://127.0.0.1:8787");
                assert_eq!(auth_token, "sk-gw");
            }
            other => panic!("expected Router, got {other:?}"),
        }
        assert!(model.is_none());
    }

    #[test]
    fn router_mode_without_cfg_falls_back_to_off() {
        let _g = ENV_LOCK.lock().unwrap();
        let state = router_state();
        let (resolution, _) = compute_provider_resolution(&state, None);
        // mode=router 但没有任何可用 router → 兜底 Off，不拒绝启动（与
        // Phantom Direct provider 同理：runtime 状态与可解析配置脱节时，
        // 让 claude 走自己的配置，而不是整 bot 卡在 refusing to launch）。
        assert!(
            matches!(resolution, ProviderResolution::Off),
            "Router mode without cfg must fall back to Off, got {resolution:?}"
        );
    }

    #[test]
    fn router_mode_empty_listen_returns_error() {
        let _g = ENV_LOCK.lock().unwrap();
        // parse 走一遍拿到合法 cfg，再把 listen 改成空——这样我们精确覆盖
        // `router_mode_uses_http_listen_url_and_first_auth_token` 的反向分支。
        let mut cfg = test_router("127.0.0.1:8787", vec!["sk-gw".to_string()]);
        cfg.listen = "".to_string();
        let state = router_state();
        let (resolution, _) = compute_provider_resolution(&state, Some(&cfg));
        match &resolution {
            ProviderResolution::Error { reason } => {
                assert!(
                    reason.contains("listen"),
                    "reason must mention listen field; got: {reason}"
                );
            }
            other => panic!("Router mode with empty listen must yield Error, got {other:?}"),
        }
    }

    /// openspec/specs/provider-management/spec.md：Error 变体经过 driver 翻译后，
    /// `resolve_spawn_overrides` 必须把 `SEBAS_PROVIDER_ERROR=<reason>`
    /// 放进 `extra_env`，spawn wrapper 据此 abort + exit(1)。
    #[test]
    fn resolve_spawn_overrides_error_emits_sebas_provider_error_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_overlay_env();
        // 触发 Error：用 router_state() 配 listen="" 的 RouterConfig。
        // 注：2e5ba41 之后 router_state()+None 改为回退 Off（旧路径不再
        // 产生 Error），所以这里走 empty-listen 分支拿 Error 变体。
        let state = router_state();
        let cfg = test_router("", vec!["sk-gw".to_string()]);
        let (env, args) = resolve_spawn_overrides(&driver(), &state, Some(&cfg));
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
    fn router_mode_emits_anthropic_env_via_driver() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = test_router("127.0.0.1:8787", vec!["sk-gw".to_string()]);
        let state = router_state();
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
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
                overlay_path.to_str().unwrap(),
            );
        }

        // --- Scenario A: Off → Off，无 env 无 args ---
        std::fs::write(
            &state_path,
            r#"{"version":2,"providers":{"test_prov":{"base_url_anthropic":"https://example.test/anthropic","api_key":"sk-test-direct"}},"deleted":[],"mode":{"kind":"off"},"default_selection":null}"#,
        )
        .unwrap();
        let st = sebas_dispatch::provider_state::load();
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
            r#"{"version":2,"providers":{"test_prov":{"base_url_anthropic":"https://example.test/anthropic","api_key":"sk-test-direct"}},"deleted":[],"mode":{"kind":"direct","provider":"test_prov"},"default_selection":{"provider":"test_prov"}}"#,
        )
        .unwrap();
        let st = sebas_dispatch::provider_state::load();
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

        // --- Scenario C: Router → Router ---
        std::fs::write(
            &state_path,
            r#"{"mode":{"kind":"router"},"default_selection":null}"#,
        )
        .unwrap();
        let st = sebas_dispatch::provider_state::load();
        let cfg = test_router("127.0.0.1:8888", vec!["sk-gw-test".to_string()]);
        match compute_provider_resolution(&st, Some(&cfg)).0 {
            ProviderResolution::Router { url, auth_token } => {
                assert_eq!(url, "http://127.0.0.1:8888");
                assert_eq!(auth_token, "sk-gw-test");
            }
            other => panic!("expected Router, got {other:?}"),
        }

        // --- Scenario D: Direct + 不存在的 provider → 兜底 Off（持久化层约定
        //     「找不到就回退 Off + warn」，契约见 openspec/specs/provider-management/spec.md）。此前这里返回
        //     `ProviderResolution::Error` → spawn wrapper `exit(1)`，用户因一个
        //     幽灵 provider 名（如泄漏的测试字面量 env-override）连 claude 都
        //     拉不起来；改成回退 Off 后启动不被阻断。---
        std::fs::write(
            &state_path,
            r#"{"mode":{"kind":"direct","provider":"nonexistent"},"default_selection":null}"#,
        )
        .unwrap();
        let st = sebas_dispatch::provider_state::load();
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
            std::env::remove_var("SEBAS_ROUTER_PROVIDER_OVERLAY");
        }
    }

    /// Direct provider 的 overlay item 同时填了 anthropic + openai base_url
    /// 且**未显式指定 `protocol` 字段**时，走「auto」默认（协议面选择契约见
    /// openspec/specs/provider-management/spec.md）：优先 Anthropic 协议面。该测试锁定 auto 默认值，避免日后
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
                  "base_url_openai_chat": "https://example.com/openai",
                  "api_key": "sk-test"
                }
              }
            }"#,
        );
        unsafe {
            std::env::set_var(
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
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
            std::env::remove_var("SEBAS_ROUTER_PROVIDER_OVERLAY");
        }
    }

    /// openspec/specs/provider-management/spec.md：overlay 里 `protocol=openai` 显式声明 +
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
                  "base_url_openai_chat": "https://example.com/openai",
                  "api_key": "sk-test",
                  "protocol": "openai"
                }
              }
            }"#,
        );
        unsafe {
            std::env::set_var(
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
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
            std::env::remove_var("SEBAS_ROUTER_PROVIDER_OVERLAY");
        }
    }

    /// openspec/specs/provider-management/spec.md.2：overlay 里 `protocol=anthropic` 但只配了
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
                  "base_url_openai_chat": "https://example.com/openai",
                  "api_key": "sk-test",
                  "protocol": "anthropic"
                }
              }
            }"#,
        );
        unsafe {
            std::env::set_var(
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
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
            std::env::remove_var("SEBAS_ROUTER_PROVIDER_OVERLAY");
        }
    }

    /// openspec/specs/provider-management/spec.md.2：overlay 里 `protocol=openai` 但只配了
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
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
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
                    reason.contains("base_url_openai_chat"),
                    "reason must explain which URL field is missing; got: {reason}"
                );
            }
            other => panic!("显式 protocol=openai 缺 base_url_openai_chat → 必须 Error；got {other:?}"),
        }
        unsafe {
            std::env::remove_var("SEBAS_ROUTER_PROVIDER_OVERLAY");
        }
    }

    /// openspec/specs/provider-management/spec.md：overlay 里 `protocol=anthropic` 显式 +
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
                  "base_url_openai_chat": "https://example.com/openai",
                  "api_key": "sk-test",
                  "protocol": "anthropic"
                }
              }
            }"#,
        );
        unsafe {
            std::env::set_var(
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
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
            std::env::remove_var("SEBAS_ROUTER_PROVIDER_OVERLAY");
        }
    }

    /// Router 模式 + router config 里有 listen 但 auth_token 是空数组：
    /// 不应 panic / 不应拒绝；URL 仍构造，auth_token 是空字符串（agent 会
    /// 在没 Bearer 的情况下调 router，router 自己拒）。这是用户故意不配
    /// auth 的合法状态。
    #[tokio::test]
    async fn router_with_empty_auth_token_still_constructs_url() {
        // parse 会读 `SEBAS_ROUTER_PROVIDER_OVERLAY`（env 优先于 config 缺省）：
        // 先持锁清 env，避免并发 overlay 用例的临时 overlay 混入本例的 parse。
        let _g = ENV_LOCK.lock().unwrap();
        clear_overlay_env();
        let raw = r#"
[router]
listen = "127.0.0.1:8787"
auth_token = []
[provider.anth-mock]
base_url_anthropic = "https://api.anthropic.com"
"#;
        let cfg = RouterConfig::parse(raw).expect("test router parses");
        let state = ProviderRuntimeState {
            mode: ProviderMode::Router,
            default_selection: None,
        };
        let (resolution, _) = compute_provider_resolution(&state, Some(&cfg));
        match resolution {
            ProviderResolution::Router { url, auth_token } => {
                assert_eq!(url, "http://127.0.0.1:8787");
                assert_eq!(
                    auth_token, "",
                    "空 auth_token 数组 → 空字符串（agent 调 router 不带 Bearer）"
                );
            }
            other => panic!("expected Router, got {other:?}"),
        }
    }

    // ---- acp-claude-model-env-cover: model cover env （openspec/changes/acp-claude-model-env-cover）----

    #[test]
    fn model_cover_env_single_model_flattens_all_tiers() {
        let env = model_cover_env(&[sebas_router::models::ModelEntry::text_only("deepseek-v4-pro[1m]")]);
        let map: std::collections::HashMap<String, String> = env.into_iter().collect();
        assert_eq!(map["ANTHROPIC_MODEL"], "deepseek-v4-pro[1m]");
        assert_eq!(map["ANTHROPIC_DEFAULT_OPUS_MODEL"], "deepseek-v4-pro[1m]");
        assert_eq!(map["ANTHROPIC_DEFAULT_SONNET_MODEL"], "deepseek-v4-pro[1m]");
        assert_eq!(map["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "deepseek-v4-pro[1m]");
        assert_eq!(map["CLAUDE_CODE_SUBAGENT_MODEL"], "deepseek-v4-pro[1m]");
        assert_eq!(map.len(), 5);
    }

    #[test]
    fn model_cover_env_multi_model_maps_strong_to_weak_and_subagent() {
        let env = model_cover_env(&[
            sebas_router::models::ModelEntry::text_only("deepseek-v4-pro[1m]"),
            sebas_router::models::ModelEntry::text_only("deepseek-v4-flash"),
        ]);
        let map: std::collections::HashMap<String, String> = env.into_iter().collect();
        assert_eq!(map["ANTHROPIC_MODEL"], "deepseek-v4-pro[1m]");
        assert_eq!(map["ANTHROPIC_DEFAULT_OPUS_MODEL"], "deepseek-v4-pro[1m]");
        assert_eq!(map["ANTHROPIC_DEFAULT_SONNET_MODEL"], "deepseek-v4-flash");
        assert_eq!(map["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "deepseek-v4-flash");
        // 当前档位策略：SUBAGENT = 最弱档 = HAIKU 值。
        assert_eq!(map["CLAUDE_CODE_SUBAGENT_MODEL"], "deepseek-v4-flash");
        assert_eq!(map.len(), 5);
    }

    #[test]
    fn model_cover_env_empty_yields_no_injection() {
        let env = model_cover_env(&[]);
        assert!(env.is_empty(), "无 models 时不应强制覆盖，child 走自己的发现");
    }

    #[test]
    fn effective_provider_models_router_returns_none() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_overlay_env();
        let state = direct_state("deepseek"); // placeholder; Router mode is the real case
        let state = ProviderRuntimeState {
            mode: ProviderMode::Router,
            default_selection: state.default_selection,
        };
        // Router 模式：模型 cover 不适用（透传是 router 的本分）。
        assert_eq!(effective_provider_models(&state, None), None);
    }

    #[test]
    fn effective_provider_models_bare_off_returns_none() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_overlay_env();
        let state = off_state();
        assert_eq!(effective_provider_models(&state, None), None);
    }

    #[test]
    fn effective_provider_models_direct_preset_reads_table() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "deepseek": {
                        "preset": "deepseek",
                        "api_key_env": "DEEPSEEK_API_KEY"
                    }
                }
            }"#,
        );
        unsafe { std::env::set_var("DEEPSEEK_API_KEY", "sk-ds-test"); }
        let state = direct_state("deepseek");
        let models = effective_provider_models(&state, None);
        unsafe { std::env::remove_var("DEEPSEEK_API_KEY"); }
        assert_eq!(models.map(|m| m.iter().map(|e| e.id.clone()).collect::<Vec<_>>()), Some(vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()]));
    }

    #[test]
    fn effective_provider_models_direct_custom_reads_item_models() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "weird": {
                        "base_url_anthropic": "https://example.test/anthropic",
                        "api_key": "sk-test",
                        "models": ["fast", "slow"]
                    }
                }
            }"#,
        );
        let state = direct_state("weird");
        let models = effective_provider_models(&state, None);
        assert_eq!(models.map(|m| m.iter().map(|e| e.id.clone()).collect::<Vec<_>>()), Some(vec!["fast".to_string(), "slow".to_string()]));
    }

    #[test]
    fn resolve_spawn_overrides_router_mode_injects_no_model_cover() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_overlay_env();
        let cfg = test_router("127.0.0.1:8787", vec!["sk-gw".to_string()]);
        let state = router_state();
        let (env, _args) = resolve_spawn_overrides(&driver(), &state, Some(&cfg));
        assert!(env.iter().any(|(k, _)| k == "ANTHROPIC_BASE_URL" && true));
        assert!(
            !env.iter().any(|(k, _)| k == "ANTHROPIC_MODEL"
                || k == "ANTHROPIC_DEFAULT_OPUS_MODEL"
                || k == "ANTHROPIC_DEFAULT_SONNET_MODEL"
                || k == "ANTHROPIC_DEFAULT_HAIKU_MODEL"
                || k == "CLAUDE_CODE_SUBAGENT_MODEL"),
            "Router 模式不应注入任何模型覆盖键；got env = {env:?}"
        );
    }

    #[test]
    fn resolve_spawn_overrides_bare_off_injects_no_model_cover() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_overlay_env();
        let state = off_state();
        let (env, _args) = resolve_spawn_overrides(&driver(), &state, None);
        assert!(env.is_empty(), "裸 Off 不注入任何 env（含模型 cover）；got env = {env:?}");
    }

    /// acp-claude-model-env-cover 端到端：Direct + overlay 含 preset →
    /// spawn env 带全部 5 个模型键，且 extra_env 里的值优先（subprocess 覆盖）。
    #[test]
    fn resolve_spawn_overrides_direct_preset_injects_5_key_model_cover() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_overlay(
            dir.path(),
            r#"{
                "providers": {
                    "deepseek": {
                        "preset": "deepseek",
                        "api_key_env": "DEEPSEEK_API_KEY",
                        "default_model": "deepseek-reasoner"
                    }
                }
            }"#,
        );
        unsafe { std::env::set_var("DEEPSEEK_API_KEY", "sk-ds-test"); }
        let state = direct_state("deepseek");
        let (env, _args) = resolve_spawn_overrides(&driver(), &state, None);
        unsafe { std::env::remove_var("DEEPSEEK_API_KEY"); }
        // preset deepseek = ["deepseek-chat", "deepseek-reasoner"]
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_MODEL" && v == "deepseek-chat"));
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_DEFAULT_OPUS_MODEL" && v == "deepseek-chat"));
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_DEFAULT_SONNET_MODEL" && v == "deepseek-reasoner"));
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_DEFAULT_HAIKU_MODEL" && v == "deepseek-reasoner"));
        assert!(env.iter().any(|(k, v)| k == "CLAUDE_CODE_SUBAGENT_MODEL" && v == "deepseek-reasoner"));
        // 端点 env 不受影响。
        assert!(env.iter().any(|(k, _)| k == "ANTHROPIC_BASE_URL"));
        assert!(env.iter().any(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN"));
    }

    /// OS env 里有残留 `ANTHROPIC_MODEL` 时，extra_env 里值覆盖（SDK .envs 语义）。
    #[test]
    fn model_cover_overrides_inherited_value_semantics() {
        let env = model_cover_env(&[sebas_router::models::ModelEntry::text_only("deepseek-v4-pro[1m]")]);
        // 模拟 SDK `.envs(&env)` 在残留 env 之上合并：残留的 `ANTHROPIC_MODEL=stale`
        // 必须由 extra_env 里的 `ANTHROPIC_MODEL=deepseek-v4-pro[1m]` 覆盖。这里
        // 断言：extra_env 里至少存在一个 `ANTHROPIC_MODEL` 键且其值是推导值。
        let stale = ("ANTHROPIC_MODEL".to_string(), "stale".to_string());
        assert!(env.iter().any(|(k, _)| *k == stale.0));
        assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_MODEL" && v == "deepseek-v4-pro[1m]"));
        assert!(!env.iter().any(|(k, v)| k == "ANTHROPIC_MODEL" && v == &stale.1));
    }
}
