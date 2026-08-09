//! gateway 配置模型与解析（spec §3）。
//!
//! provider 统一放在顶层 `[provider.<name>]`（run 与 gateway 共用），不再支持
//! 复数 `[providers.*]` 或 `[gateway.providers.*]` 旧写法。
//! 只有顶层 provider、无 `[gateway]` 段时，其余字段全部走默认值。
//!
//! provider 支持「名称即 preset」（可选显式 `preset = "..."` 别名）：
//! anthropic / openai / deepseek / kimi / glm / minimax / ark / dashscope /
//! gemini 自带 `protocol` / `base_url` / `api_key_env` 惯例默认，显式字段永远
//! 覆盖；双协议 provider 必须显式 `protocol`（不猜）。
//! 下游 key 支持 `key_env`（从 env 读，不落盘/不落日志），与 `key` 至少二选一、
//! 不可同时设置。

use serde::Deserialize;
use std::collections::HashMap;

use crate::error::{GatewayError, Result};
use crate::proto::Protocol;

/// 顶层包装：容忍同一 config.toml 中的 `[feishu]` / `[acp.*]` 等无关段，
/// 只取 `[gateway]`。有意不复用 root `Config::parse`——gateway 的运行边界
/// 与配置 schema 独立于 sebas 主进程（spec §3）。
///
/// provider 采用「raw → resolved」两段解析：TOML 先落到 `RawGatewayConfig`
/// （provider 字段全 Option），再经 `resolve_providers` 应用 preset 得到对外
/// `GatewayConfig`。对外结构不含 preset 痕迹，routing/proxy 零改动。
#[derive(Deserialize)]
struct GatewayFile {
    #[serde(default)]
    gateway: Option<RawGatewayConfig>,
    /// 顶层 provider 表（唯一写法）：`[provider.<name>]`，与 `run` 共用。
    #[serde(default)]
    provider: HashMap<String, RawProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: u64,
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_read_timeout_secs")]
    pub read_timeout_secs: u64,
    #[serde(default = "default_usage_file")]
    pub usage_file: String,
    /// debug 模式（`--debug` 启动参数触发，parse 后注入内置 test provider）：
    /// 由 gateway 自身应答（固定文字 + 回显输入），不转发外部上游。
    pub debug: bool,
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub keys: Vec<KeyConfig>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub routes: Vec<RouteGroup>,
}

fn default_listen() -> String {
    "127.0.0.1:8787".into()
}
fn default_max_body_bytes() -> u64 {
    67_108_864 // 64 MiB
}
fn default_connect_timeout_secs() -> u64 {
    10
}
fn default_read_timeout_secs() -> u64 {
    600
}
fn default_usage_file() -> String {
    // $HOME/.sebas/ works on both Unix and Windows (tilde expansion below
    // resolves it through dirs::home_dir()).
    "~/.sebas/gateway-usage.jsonl".into()
}

/// 下游客户端 key：鉴权身份 + 限流/配额参数。`key` 是网关签发给客户端的
/// 令牌（非上游 provider 密钥）；解析后非空且全局不重复。
/// `key_env` 是可选替代：从 env 读下游 key（不落盘/不落日志），与 `key`
/// 至少二选一、不可同时设置。
#[derive(Debug, Clone, Deserialize)]
pub struct KeyConfig {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub key_env: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub rpm: Option<u32>,
    #[serde(default)]
    pub daily_token_quota: Option<u64>,
    #[serde(default)]
    pub allow_models: Vec<String>,
    /// 该 key 级别的默认 provider，覆盖全局 `default_provider`。
    #[serde(default)]
    pub default_provider: Option<String>,
}

/// 上游 provider。`api_key_env` 优先（密钥只从 env 读，不落盘/不落日志）；
/// `api_key` 明文仅测试用（resolve 时 warn）。两者均无 → Config 错误。
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub protocol: Protocol,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model_map: HashMap<String, String>,
}

/// 路由：model（可含 glob）→ 有序 provider 列表。数组顺序即优先级，
/// 第一个为主 provider；后续为备选。
/// TODO(故障转移): 当前路由只取第一个，切换逻辑见 routing.rs 的 TODO。
#[derive(Debug, Clone, Deserialize)]
pub struct RouteGroup {
    pub model: String,
    pub providers: Vec<String>,
}

/// TOML 原始形态的 `[gateway]` 段：字段全 Option / 默认值，供 preset 解析。
/// 与对外 `GatewayConfig` 的区别仅在 `providers`（raw 字段可缺省）。
#[derive(Deserialize)]
struct RawGatewayConfig {
    #[serde(default = "default_listen")]
    listen: String,
    #[serde(default = "default_max_body_bytes")]
    max_body_bytes: u64,
    #[serde(default = "default_connect_timeout_secs")]
    connect_timeout_secs: u64,
    #[serde(default = "default_read_timeout_secs")]
    read_timeout_secs: u64,
    #[serde(default = "default_usage_file")]
    usage_file: String,
    #[serde(default)]
    default_provider: Option<String>,
    #[serde(default)]
    keys: Vec<KeyConfig>,
    #[serde(default)]
    /// `[gateway.routes]`：`model = ["provider", ...]`，数组顺序 = 优先级。
    routes: HashMap<String, Vec<String>>,
}

/// TOML 原始形态的 provider 段：字段全 Option，preset 填充后再收敛成
/// 对外 `ProviderConfig`（protocol/base_url 必填）。
#[derive(Deserialize)]
struct RawProviderConfig {
    /// 显式 preset 别名；缺省时「名称即 preset」。
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    protocol: Option<Protocol>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_key_env: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    model_map: HashMap<String, String>,
}

/// provider 惯例默认（spec §6 Provider 格局调研 + 2026-08-04/07 端点探测）。
/// 双协议 provider（anthropic + openai 端点都有）必须显式 `protocol`，不猜。
/// 默认 env 名均可被 `api_key_env` 覆盖。
struct ProviderPreset {
    name: &'static str,
    /// anthropic 协议端点；`None` = 该 preset 不提供 anthropic 端点。
    anthropic_base_url: Option<&'static str>,
    /// openai 协议端点；`None` = 该 preset 不提供 openai 端点。
    openai_base_url: Option<&'static str>,
    /// 默认 env 变量名（可被 `api_key_env` 覆盖）。
    api_key_env: &'static str,
}

const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "anthropic",
        anthropic_base_url: Some("https://api.anthropic.com"),
        openai_base_url: None,
        api_key_env: "ANTHROPIC_API_KEY",
    },
    ProviderPreset {
        name: "openai",
        anthropic_base_url: None,
        openai_base_url: Some("https://api.openai.com/v1"),
        api_key_env: "OPENAI_API_KEY",
    },
    ProviderPreset {
        name: "deepseek",
        anthropic_base_url: Some("https://api.deepseek.com/anthropic"),
        openai_base_url: Some("https://api.deepseek.com/v1"),
        api_key_env: "DEEPSEEK_API_KEY",
    },
    ProviderPreset {
        name: "kimi",
        anthropic_base_url: Some("https://api.moonshot.cn/anthropic"),
        openai_base_url: Some("https://api.moonshot.cn/v1"),
        api_key_env: "MOONSHOT_API_KEY",
    },
    ProviderPreset {
        name: "glm",
        anthropic_base_url: Some("https://open.bigmodel.cn/api/anthropic"),
        openai_base_url: Some("https://open.bigmodel.cn/api/paas/v4"),
        api_key_env: "ZHIPU_API_KEY",
    },
    ProviderPreset {
        name: "minimax",
        anthropic_base_url: Some("https://api.minimaxi.com/anthropic"),
        openai_base_url: Some("https://api.minimaxi.com/v1"),
        api_key_env: "MINIMAX_API_KEY",
    },
    ProviderPreset {
        name: "ark",
        anthropic_base_url: Some("https://ark.cn-beijing.volces.com/api/coding"),
        openai_base_url: Some("https://ark.cn-beijing.volces.com/api/coding/v3"),
        api_key_env: "ARK_API_KEY",
    },
    ProviderPreset {
        name: "dashscope",
        anthropic_base_url: None,
        openai_base_url: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        api_key_env: "DASHSCOPE_API_KEY",
    },
    ProviderPreset {
        name: "gemini",
        anthropic_base_url: None,
        openai_base_url: Some("https://generativelanguage.googleapis.com/v1beta/openai"),
        api_key_env: "GEMINI_API_KEY",
    },
];

fn find_preset(name: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS.iter().find(|p| p.name == name)
}

/// raw → resolved：把每个 provider 收敛成对外 `ProviderConfig`。
/// - 名称即 preset（或显式 `preset = "..."` 别名）：先填预设默认值，
///   显式字段（protocol/base_url/api_key_env）永远覆盖；
/// - 双协议 preset 未写 `protocol` → 配置错误（不猜）；
/// - 单协议 preset 显式写了对方协议 → 配置错误（preset 不提供该端点）；
/// - 无 preset（自定义 provider）→ 维持现状：protocol + base_url 必填。
fn resolve_providers(
    raw: HashMap<String, RawProviderConfig>,
) -> Result<HashMap<String, ProviderConfig>> {
    let mut out = HashMap::with_capacity(raw.len());
    for (name, r) in raw {
        let preset_name = r.preset.as_deref().unwrap_or(&name);
        let preset = find_preset(preset_name);
        let (protocol, base_url) = match preset {
            Some(p) => resolve_preset_protocol_base(p, &name, preset_name, r.protocol, r.base_url)?,
            None => {
                let protocol = r.protocol.ok_or_else(|| {
                    GatewayError::Config(format!(
                        "provider.{name}.protocol 不能为空（自定义 provider 无 preset，需显式 protocol）"
                    ))
                })?;
                let base_url = r.base_url.unwrap_or_default();
                (protocol, base_url)
            }
        };
        // 默认 env 名只在「未显式指定任何 key 来源」时注入：显式 `api_key_env`
        // 优先；显式 `api_key`（明文，仅测试用）则不应被 preset 默认 env 覆盖。
        let api_key_env = r.api_key_env.or_else(|| {
            if r.api_key.is_none() {
                preset.map(|p| p.api_key_env.to_string())
            } else {
                None
            }
        });
        out.insert(
            name,
            ProviderConfig {
                protocol,
                base_url,
                api_key_env,
                api_key: r.api_key,
                model_map: r.model_map,
            },
        );
    }
    Ok(out)
}

/// 按 preset 解析 `(protocol, base_url)`。显式字段优先；歧义/缺失端点报错。
fn resolve_preset_protocol_base(
    p: &ProviderPreset,
    name: &str,
    preset_name: &str,
    explicit_protocol: Option<Protocol>,
    explicit_base_url: Option<String>,
) -> Result<(Protocol, String)> {
    let protocol = match explicit_protocol {
        Some(proto) => proto,
        None => match (p.anthropic_base_url, p.openai_base_url) {
            (Some(_), None) => Protocol::Anthropic,
            (None, Some(_)) => Protocol::OpenAi,
            _ => {
                return Err(GatewayError::Config(format!(
                    "provider.{name}: preset '{preset_name}' 同时提供 anthropic/openai 端点，必须显式 protocol"
                )));
            }
        },
    };
    let base_url = match explicit_base_url {
        Some(b) => b,
        None => match protocol {
            Protocol::Anthropic => p.anthropic_base_url,
            Protocol::OpenAi => p.openai_base_url,
        }
        .ok_or_else(|| {
            GatewayError::Config(format!(
                "provider.{name}: preset '{preset_name}' 不提供 {} 端点",
                protocol.as_str()
            ))
        })?
        .to_string(),
    };
    Ok((protocol, base_url))
}

impl GatewayConfig {
    /// 解析顺序对齐 root house style（src/config.rs）：
    /// toml → preset 填充（raw → resolved）→ env 覆盖（`SEBAS_GATEWAY_LISTEN`）
    /// → validate → tilde 展开（`usage_file`）。
    pub fn parse(raw: &str) -> Result<Self> {
        let file: GatewayFile =
            toml::from_str(raw).map_err(|e| GatewayError::Config(format!("toml parse: {e}")))?;

        // provider 唯一来源：顶层 `[provider.*]`。
        let providers = file.provider;
        let raw_cfg = file.gateway;
        if raw_cfg.is_none() && providers.is_empty() {
            return Err(GatewayError::Config(
                "config 缺少 [gateway] 段或 [provider] 段".into(),
            ));
        }
        let providers = resolve_providers(providers)?;

        let mut cfg = match raw_cfg {
            Some(g) => {
                // `[gateway.routes]` map → 有序 RouteGroup 列表。TOML map 本身
                // 无配置序保证，按 model 名排序保证确定性（glob 撞车时字典序
                // 首个命中；精确键天然唯一）。
                let mut routes: Vec<RouteGroup> = g
                    .routes
                    .into_iter()
                    .map(|(model, providers)| RouteGroup { model, providers })
                    .collect();
                routes.sort_by(|a, b| a.model.cmp(&b.model));
                GatewayConfig {
                    listen: g.listen,
                    max_body_bytes: g.max_body_bytes,
                    connect_timeout_secs: g.connect_timeout_secs,
                    read_timeout_secs: g.read_timeout_secs,
                    usage_file: g.usage_file,
                    debug: false,
                    default_provider: g.default_provider,
                    keys: g.keys,
                    providers,
                    routes,
                }
            }
            // 只有顶层 `[provider.*]`：无 [gateway] 段，其余字段全部走默认。
            None => GatewayConfig {
                listen: default_listen(),
                max_body_bytes: default_max_body_bytes(),
                connect_timeout_secs: default_connect_timeout_secs(),
                read_timeout_secs: default_read_timeout_secs(),
                usage_file: default_usage_file(),
                debug: false,
                default_provider: None,
                keys: Vec::new(),
                providers,
                routes: Vec::new(),
            },
        };
        cfg.apply_env_overrides();
        cfg.validate()?;
        Ok(cfg.with_expanded_paths())
    }

    /// env 覆盖（spec §6.3，优先级高于 TOML）。空值忽略，避免
    /// `SEBAS_GATEWAY_LISTEN=` 抹掉已配置的 listen。在 validate 之前运行。
    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("SEBAS_GATEWAY_LISTEN")
            && !v.is_empty()
        {
            self.listen = v;
        }
    }

    fn validate(&self) -> Result<()> {
        if self.providers.is_empty() {
            return Err(GatewayError::Config("provider 不能为空".into()));
        }
        for (name, p) in &self.providers {
            if p.base_url.is_empty() {
                return Err(GatewayError::Config(format!(
                    "provider.{name}.base_url 不能为空"
                )));
            }
        }
        if let Some(dp) = &self.default_provider
            && !self.providers.contains_key(dp)
        {
            return Err(GatewayError::Config(format!(
                "gateway.default_provider 引用了未定义的 provider '{dp}'"
            )));
        }
        for r in &self.routes {
            if r.providers.is_empty() {
                return Err(GatewayError::Config(format!(
                    "gateway.routes.{} 的 provider 列表不能为空",
                    r.model
                )));
            }
            for p in &r.providers {
                if !self.providers.contains_key(p) {
                    return Err(GatewayError::Config(format!(
                        "gateway.routes.{} 引用了未定义的 provider '{p}'",
                        r.model
                    )));
                }
            }
        }
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (i, k) in self.keys.iter().enumerate() {
            let key_set = !k.key.is_empty();
            match (key_set, k.key_env.is_some()) {
                (false, false) => {
                    return Err(GatewayError::Config(format!(
                        "gateway.keys[{i}] 必须配置 key 或 key_env（两者皆空）"
                    )));
                }
                (true, true) => {
                    return Err(GatewayError::Config(format!(
                        "gateway.keys[{i}] 不能同时配置 key 和 key_env"
                    )));
                }
                _ => {}
            }
            if key_set {
                if let Some(&first) = seen.get(k.key.as_str()) {
                    return Err(GatewayError::Config(format!(
                        "gateway.keys[{i}] 与 keys[{first}] 的 key 重复"
                    )));
                }
                seen.insert(k.key.as_str(), i);
            }
            if let Some(dp) = &k.default_provider
                && !self.providers.contains_key(dp)
            {
                return Err(GatewayError::Config(format!(
                    "gateway.keys[{i}].default_provider 引用了未定义的 provider '{dp}'"
                )));
            }
        }
        Ok(())
    }

    fn with_expanded_paths(mut self) -> Self {
        self.usage_file = expand_tilde(&self.usage_file);
        self
    }

    /// 解析下游 key（含 `key_env` 环境变量读取），返回 `呈现 key → KeyConfig` 表。
    /// - `key` 非空 → 直接用；
    /// - `key_env` 指向的 env 变量必须存在且非空（错误信息只含变量名，绝不含值）；
    /// - 解析后的 key 全局不重复（`key_env` 的重复只能在运行时发现）。
    ///
    /// 返回值中的 `KeyConfig.key` 已被替换为解析后的真实密钥（`key_env` 场景），
    /// 保证 `KeyIdentity.config.key` / quota 记账用的是真实呈现值。
    pub fn resolve_keys(&self) -> Result<HashMap<String, KeyConfig>> {
        let mut out = HashMap::with_capacity(self.keys.len());
        let mut seen: HashMap<String, usize> = HashMap::with_capacity(self.keys.len());
        for (i, k) in self.keys.iter().enumerate() {
            let resolved = if !k.key.is_empty() {
                k.key.clone()
            } else if let Some(env_var) = &k.key_env {
                match std::env::var(env_var) {
                    Ok(v) if !v.is_empty() => v,
                    // debug 模式：缺失/空 env 的 key 直接跳过，不阻塞启动
                    // （debug 下鉴权本身也跳过，见 auth.rs）。
                    _ if self.debug => continue,
                    _ => {
                        return Err(GatewayError::Config(format!(
                            "gateway.keys[{i}].key_env 指向的环境变量 '{env_var}' 未设置或为空"
                        )));
                    }
                }
            } else {
                // validate() 已保证至少一个；防御性错误（不含值）。
                return Err(GatewayError::Config(format!(
                    "gateway.keys[{i}] 未配置 key 或 key_env"
                )));
            };
            if let Some(&first) = seen.get(&resolved) {
                return Err(GatewayError::Config(format!(
                    "gateway.keys[{i}] 与 keys[{first}] 解析后的 key 重复"
                )));
            }
            let mut kc = k.clone();
            kc.key = resolved.clone();
            seen.insert(resolved.clone(), i);
            out.insert(resolved, kc);
        }
        Ok(out)
    }

    /// 解析每个 provider 的上游 api key：
    /// - `api_key_env` 指向的 env 变量必须存在且非空（错误信息只含变量名，绝不含 key 值）；
    /// - 否则回退明文 `api_key`（仅测试用，emit warn）；
    /// - 两者都缺 → Config 错误。
    pub fn resolve_api_keys(&self) -> Result<HashMap<String, String>> {
        let mut out = HashMap::with_capacity(self.providers.len());
        for (name, p) in &self.providers {
            // 内置 test provider（debug 模式注入）不触达外部上游，无需 key。
            if self.debug && name == "test" {
                continue;
            }
            let key = if let Some(env_var) = &p.api_key_env {
                match std::env::var(env_var) {
                    Ok(v) if !v.is_empty() => v,
                    _ => {
                        return Err(GatewayError::Config(format!(
                            "provider.{name}.api_key_env 指向的环境变量 '{env_var}' 未设置或为空"
                        )));
                    }
                }
            } else if let Some(plain) = &p.api_key {
                tracing::warn!(
                    "gateway provider '{name}' 使用明文 api_key（仅测试用，生产请改用 api_key_env）"
                );
                plain.clone()
            } else {
                return Err(GatewayError::Config(format!(
                    "provider.{name} 未配置 api_key_env 或 api_key"
                )));
            };
            out.insert(name.clone(), key);
        }
        Ok(out)
    }

    /// `--debug`：配置解析完成后注入内置 test provider——provider 名 `test`、
    /// base_url 指向 gateway 自身（哨兵值，不实际拨号），并加一条
    /// `test → test` 路由（插到路由表最前，debug 模式下优先于用户配置）。
    /// 路由命中后由 proxy 短路应答，见 `test_provider` 模块。
    pub fn enable_debug_test_provider(&mut self) {
        self.debug = true;
        tracing::debug!("debug mode: injecting built-in test provider");
        self.providers.insert(
            "test".to_string(),
            ProviderConfig {
                protocol: Protocol::Anthropic,
                base_url: "gateway://self".to_string(),
                api_key_env: None,
                api_key: None,
                model_map: HashMap::new(),
            },
        );
        if !self
            .routes
            .iter()
            .any(|r| r.model == "test" && r.providers.len() == 1 && r.providers[0] == "test")
        {
            self.routes.insert(
                0,
                RouteGroup {
                    model: "test".to_string(),
                    providers: vec!["test".to_string()],
                },
            );
        }
    }
}

/// tilde 展开（与 root `src/config.rs` 同款 let-chain 形式）。
fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into_owned();
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::proto::Protocol;

    // 所有 config 测试都经此锁串行：env 覆盖测试会 set/remove
    // `SEBAS_GATEWAY_LISTEN`，单进程内并行跑会与其他调用 parse 的测试竞争。
    static LOCK: Mutex<()> = Mutex::new(());

    const FULL_EXAMPLE: &str = r#"
[gateway]
default_provider = "anthropic"

[[gateway.keys]]
key = "sk-gw-local-dev"
name = "claude-code"
rpm = 600
daily_token_quota = 50_000_000
allow_models = ["claude-*", "deepseek-*"]

[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"

[provider.deepseek]
protocol = "anthropic"
base_url = "https://api.deepseek.com/anthropic"
api_key_env = "DEEPSEEK_API_KEY"

[gateway.routes]
"claude-*" = ["anthropic"]
"deepseek-*" = ["deepseek"]
"#;

    #[test]
    fn parses_full_example_with_defaults() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let cfg = GatewayConfig::parse(FULL_EXAMPLE).expect("full example should parse");
        assert_eq!(cfg.listen, "127.0.0.1:8787");
        assert_eq!(cfg.max_body_bytes, 67_108_864);
        assert_eq!(cfg.connect_timeout_secs, 10);
        assert_eq!(cfg.read_timeout_secs, 600);
        let expected_suffix = std::path::Path::new(".sebas").join("gateway-usage.jsonl");
        let usage_path = std::path::Path::new(&cfg.usage_file);
        assert!(
            usage_path.ends_with(expected_suffix),
            "usage_file {:?} should end with the .sebas suffix",
            cfg.usage_file
        );
        assert_eq!(cfg.default_provider.as_deref(), Some("anthropic"));
        assert_eq!(cfg.keys.len(), 1);
        assert_eq!(cfg.keys[0].key, "sk-gw-local-dev");
        assert_eq!(cfg.keys[0].name, "claude-code");
        assert_eq!(cfg.keys[0].rpm, Some(600));
        assert_eq!(cfg.keys[0].daily_token_quota, Some(50_000_000));
        assert_eq!(cfg.keys[0].allow_models, vec!["claude-*", "deepseek-*"]);
        assert_eq!(cfg.providers.len(), 2);
        let anth = cfg.providers.get("anthropic").expect("anthropic provider");
        assert_eq!(anth.protocol, Protocol::Anthropic);
        assert_eq!(anth.base_url, "https://api.anthropic.com");
        assert_eq!(anth.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
        assert_eq!(cfg.routes.len(), 2);
        assert_eq!(cfg.routes[0].model, "claude-*");
        assert_eq!(cfg.routes[0].providers, vec!["anthropic"]);
        assert_eq!(cfg.routes[1].providers, vec!["deepseek"]);
    }

    #[test]
    fn tolerates_unrelated_sections() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = format!("{FULL_EXAMPLE}\n[feishu]\napp_id = \"x\"\napp_secret = \"y\"\n");
        let cfg = GatewayConfig::parse(&raw).expect("should parse with [feishu] present");
        assert_eq!(cfg.providers.len(), 2);
    }

    #[test]
    fn missing_gateway_section_is_config_error() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let err =
            GatewayConfig::parse("[feishu]\napp_id = \"x\"\n").expect_err("missing [gateway]");
        let msg = err.to_string();
        assert!(
            msg.contains("[gateway]"),
            "error should mention [gateway]: {msg}"
        );
    }

    #[test]
    fn env_overrides_listen() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_LISTEN", "0.0.0.0:9999");
        }
        let cfg = GatewayConfig::parse(FULL_EXAMPLE).expect("parse");
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        assert_eq!(cfg.listen, "0.0.0.0:9999");
    }

    #[test]
    fn route_referencing_unknown_provider_errors() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key = "sk-test"
[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
[gateway.routes]
"gpt-*" = ["openai"]
"#;
        let err = GatewayConfig::parse(raw).expect_err("unknown provider should error");
        let msg = err.to_string();
        assert!(
            msg.contains("openai"),
            "error should name the unknown provider: {msg}"
        );
    }

    #[test]
    fn usage_file_tilde_is_expanded() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let home = dirs::home_dir().expect("HOME must be set for this test");
        let raw = r#"
[gateway]
usage_file = "~/sebas/gateway-usage.jsonl"
[[gateway.keys]]
key = "sk-test"
[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
"#;
        let cfg = GatewayConfig::parse(raw).expect("parse");
        assert_eq!(
            cfg.usage_file,
            home.join("sebas/gateway-usage.jsonl")
                .to_string_lossy()
                .into_owned()
        );
    }

    // -------------------- provider preset --------------------

    #[test]
    fn preset_fills_defaults_and_explicit_fields_override() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key = "sk-test"

# 名称即 preset：只写 protocol，base_url/api_key_env 自动填充
[provider.deepseek]
protocol = "anthropic"

# 显式字段覆盖 preset 默认
[provider.openai]
base_url = "http://localhost:9099/v1"
api_key_env = "MY_OPENAI_KEY"
"#;
        let cfg = GatewayConfig::parse(raw).expect("preset config should parse");

        let ds = cfg.providers.get("deepseek").expect("deepseek provider");
        assert_eq!(ds.protocol, Protocol::Anthropic);
        assert_eq!(ds.base_url, "https://api.deepseek.com/anthropic");
        assert_eq!(ds.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));

        let oai = cfg.providers.get("openai").expect("openai provider");
        // 单协议 preset：protocol 缺省自动填 openai
        assert_eq!(oai.protocol, Protocol::OpenAi);
        // 显式 base_url / api_key_env 覆盖 preset
        assert_eq!(oai.base_url, "http://localhost:9099/v1");
        assert_eq!(oai.api_key_env.as_deref(), Some("MY_OPENAI_KEY"));
    }

    #[test]
    fn preset_dual_protocol_requires_explicit_protocol() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key = "sk-test"
[provider.deepseek]
"#;
        let err = GatewayConfig::parse(raw).expect_err("dual-protocol preset without protocol");
        let msg = err.to_string();
        assert!(
            msg.contains("deepseek") && msg.contains("protocol"),
            "error should name provider and require protocol: {msg}"
        );
    }

    #[test]
    fn preset_alias_reuses_table_defaults() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key = "sk-test"
[provider.my-openai]
preset = "openai"
"#;
        let cfg = GatewayConfig::parse(raw).expect("preset alias should parse");
        let p = cfg.providers.get("my-openai").expect("aliased provider");
        assert_eq!(p.protocol, Protocol::OpenAi);
        assert_eq!(p.base_url, "https://api.openai.com/v1");
        assert_eq!(p.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
    }

    #[test]
    fn preset_single_protocol_with_foreign_protocol_errors() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key = "sk-test"
[provider.openai]
protocol = "anthropic"
"#;
        let err = GatewayConfig::parse(raw).expect_err("openai preset with anthropic protocol");
        let msg = err.to_string();
        assert!(
            msg.contains("openai") && msg.contains("anthropic"),
            "error should name preset and missing endpoint: {msg}"
        );
    }

    #[test]
    fn custom_provider_without_protocol_errors() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key = "sk-test"
[provider.my-custom]
base_url = "http://localhost:1234"
"#;
        let err = GatewayConfig::parse(raw).expect_err("custom provider without protocol");
        assert!(
            err.to_string().contains("protocol"),
            "custom provider must require explicit protocol: {err}"
        );
    }

    #[test]
    fn preset_explicit_api_key_skips_default_env() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        // anthropic preset + 显式明文 api_key（仅测试用）：不得再注入
        // ANTHROPIC_API_KEY 默认 env，否则 resolve_api_keys 会误读 env。
        let raw = r#"
[gateway]
[[gateway.keys]]
key = "sk-test"
[provider.anthropic]
api_key = "test-key"
"#;
        let cfg = GatewayConfig::parse(raw).expect("preset + api_key should parse");
        let p = cfg.providers.get("anthropic").expect("anthropic provider");
        assert_eq!(p.protocol, Protocol::Anthropic);
        assert_eq!(p.base_url, "https://api.anthropic.com");
        assert_eq!(
            p.api_key_env, None,
            "explicit api_key must not get preset env"
        );
        assert_eq!(p.api_key.as_deref(), Some("test-key"));
    }

    // -------------------- 顶层 [provider.*] --------------------

    #[test]
    fn top_level_provider_table_parses_with_gateway_section() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
default_provider = "deepseek"

[[gateway.keys]]
key = "sk-test"

[provider.deepseek]
protocol = "anthropic"
"#;
        let cfg = GatewayConfig::parse(raw).expect("top-level provider should parse");
        assert_eq!(cfg.default_provider.as_deref(), Some("deepseek"));
        assert_eq!(cfg.keys.len(), 1);
        let ds = cfg.providers.get("deepseek").expect("deepseek provider");
        assert_eq!(ds.protocol, Protocol::Anthropic);
        assert_eq!(ds.base_url, "https://api.deepseek.com/anthropic");
        assert_eq!(ds.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
    }

    #[test]
    fn top_level_provider_without_gateway_section_uses_defaults() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        // run --gateway 场景：只有顶层 [provider.*]，无 [gateway] 段。
        let raw = r#"
[feishu]
app_id = "x"

[provider.anthropic]
"#;
        let cfg = GatewayConfig::parse(raw).expect("provider-only config should parse");
        assert_eq!(cfg.listen, "127.0.0.1:8787");
        assert_eq!(cfg.keys.len(), 0);
        assert_eq!(cfg.routes.len(), 0);
        let p = cfg.providers.get("anthropic").expect("anthropic provider");
        assert_eq!(p.protocol, Protocol::Anthropic);
        assert_eq!(p.base_url, "https://api.anthropic.com");
        assert_eq!(p.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn top_level_provider_table_with_explicit_base_url_and_plain_key() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        // 顶层 `[provider.*]`：preset 自动填充 + 显式 base_url/api_key 覆盖。
        let raw = r#"
[provider.deepseek]
protocol = "anthropic"

[provider.ark]
protocol = "anthropic"
base_url = "https://ark.cn-beijing.volces.com/api/plan"
api_key = "test-ark-key"
"#;
        let cfg = GatewayConfig::parse(raw).expect("top-level provider table should parse");
        let ds = cfg.providers.get("deepseek").expect("deepseek provider");
        assert_eq!(ds.protocol, Protocol::Anthropic);
        assert_eq!(ds.base_url, "https://api.deepseek.com/anthropic");
        assert_eq!(ds.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));

        let ark = cfg.providers.get("ark").expect("ark provider");
        assert_eq!(ark.base_url, "https://ark.cn-beijing.volces.com/api/plan");
        assert_eq!(ark.api_key.as_deref(), Some("test-ark-key"));
        assert_eq!(ark.api_key_env, None);
    }

    // -------------------- key_env --------------------

    #[test]
    fn key_env_only_parses_and_resolves_from_env() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
            std::env::set_var("SEBAS_GATEWAY_TEST_KEY_ENV", "sk-from-env-123");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key_env = "SEBAS_GATEWAY_TEST_KEY_ENV"
name = "env-key"
[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
"#;
        let cfg = GatewayConfig::parse(raw).expect("key_env config should parse");
        // env 变量仍在场时解析（parse 之后、remove 之前）。
        let keys = cfg.resolve_keys().expect("resolve_keys");
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_TEST_KEY_ENV");
        }
        let kc = keys.get("sk-from-env-123").expect("env key in map");
        assert_eq!(kc.name, "env-key");
        assert_eq!(kc.key, "sk-from-env-123");
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn key_env_missing_env_var_errors_without_value() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
            std::env::remove_var("SEBAS_GATEWAY_TEST_MISSING_KEY");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key_env = "SEBAS_GATEWAY_TEST_MISSING_KEY"
[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
"#;
        let cfg = GatewayConfig::parse(raw).expect("key_env config should parse");
        let err = cfg.resolve_keys().expect_err("missing env must error");
        let msg = err.to_string();
        assert!(
            msg.contains("SEBAS_GATEWAY_TEST_MISSING_KEY"),
            "error should name the env var: {msg}"
        );
        assert!(
            !msg.contains("sk-") && !msg.contains("secret"),
            "error must not echo env values: {msg}"
        );
    }

    #[test]
    fn key_and_key_env_both_set_errors() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key = "sk-plain"
key_env = "SEBAS_GATEWAY_TEST_KEY_ENV"
[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
"#;
        let err = GatewayConfig::parse(raw).expect_err("key + key_env must error");
        assert!(
            err.to_string().contains("key_env"),
            "error should mention key_env conflict: {err}"
        );
    }

    #[test]
    fn key_neither_set_errors() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
name = "no-key"
[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
"#;
        let err = GatewayConfig::parse(raw).expect_err("neither key nor key_env must error");
        assert!(
            err.to_string().contains("key_env"),
            "error should mention key/key_env requirement: {err}"
        );
    }

    #[test]
    fn enable_debug_test_provider_injects_test_provider_and_route() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key = "sk-test"
[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
"#;
        let mut cfg = GatewayConfig::parse(raw).expect("parse");
        assert!(!cfg.debug);
        cfg.enable_debug_test_provider();

        assert!(cfg.debug);
        let test = cfg.providers.get("test").expect("test provider injected");
        assert_eq!(test.base_url, "gateway://self");
        assert_eq!(test.api_key, None);
        assert_eq!(test.api_key_env, None);
        assert!(
            cfg.routes
                .iter()
                .any(|r| r.model == "test" && r.providers.len() == 1 && r.providers[0] == "test"),
            "test → test route must be injected"
        );
        assert_eq!(
            cfg.routes[0].model, "test",
            "debug route should lead the table"
        );

        // resolve_api_keys 跳过内置 test provider（不触达上游，无需 key）。
        let keys = cfg.resolve_api_keys().expect("resolve_api_keys");
        assert!(!keys.contains_key("test"));
        assert!(keys.contains_key("anthropic"));

        // 幂等：重复注入不产生重复路由。
        cfg.enable_debug_test_provider();
        assert_eq!(
            cfg.routes
                .iter()
                .filter(|r| r.model == "test" && r.providers.len() == 1 && r.providers[0] == "test")
                .count(),
            1
        );
    }

    #[test]
    fn debug_mode_skips_missing_key_env_at_resolve() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
            std::env::remove_var("SEBAS_GATEWAY_TEST_MISSING_KEY");
        }
        let raw = r#"
[gateway]
[[gateway.keys]]
key_env = ""
[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
"#;
        let mut cfg = GatewayConfig::parse(raw).expect("parse");
        cfg.enable_debug_test_provider();
        // debug 模式：key_env 留空（或指向未设置的变量）直接跳过而不是报错。
        let keys = cfg
            .resolve_keys()
            .expect("debug must tolerate missing key_env");
        assert!(
            keys.is_empty(),
            "missing-env key must be skipped in debug mode"
        );
    }
}
