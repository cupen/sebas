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
//! 下游客户端鉴权用 `[gateway] auth_token`（单个字符串或字符串数组），只做
//! Bearer/x-api-key 匹配，无 per-key 限流/配额/模型白名单等特性。
//! 不配置则网关不校验下游 token（裸奔，启动时 warn）。

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
    /// bot 侧 `/provider` 命令写入的 provider 变更文件（delta + 删除墓碑）。
    /// parse 时与顶层 `[provider.*]` 合并：overlay 里
    /// 的同名条目覆盖/新增，deleted 墓碑移除条目。config.toml 保持只读。
    #[serde(default = "default_provider_overlay")]
    pub provider_overlay: String,
    #[serde(default)]
    pub default_provider: Option<String>,
    /// 下游客户端鉴权 token：单个字符串或字符串数组（TOML 两者都接受）。
    /// 只做 Bearer/x-api-key 匹配，无 per-key 限流/配额/模型白名单等特性。
    /// 不配置则不校验（裸奔，启动时 warn）。
    #[serde(default, deserialize_with = "de_auth_token")]
    pub auth_token: Vec<String>,
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
fn default_provider_overlay() -> String {
    "~/.sebas/providers.json".into()
}

/// 上游 provider。`api_key_env` 优先（密钥只从 env 读，不落盘/不落日志）；
/// `api_key` 明文仅测试用（resolve 时 warn）。两者均无 → Config 错误。
///
/// `base_url_anthropic` / `base_url_openai` 各自独立：同一 provider 可同时
/// 暴露两种协议（如 deepseek、ark），各自指向不同的上游路径；请求按协议取
/// 对应 URL，缺位 → `ProtocolMismatch`。至少一项必填。
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub base_url_anthropic: Option<String>,
    #[serde(default)]
    pub base_url_openai: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model_map: HashMap<String, String>,
}

/// 给定请求协议返回对应上游 URL；两项都为 None 视为未配置。
impl ProviderConfig {
    pub fn url_for(&self, proto: Protocol) -> Option<&str> {
        match proto {
            Protocol::Anthropic => self.base_url_anthropic.as_deref(),
            Protocol::OpenAi => self.base_url_openai.as_deref(),
        }
    }
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
    #[serde(default, deserialize_with = "de_auth_token")]
    auth_token: Vec<String>,
    #[serde(default)]
    /// `[gateway.routes]`：`model = ["provider", ...]`，数组顺序 = 优先级。
    routes: HashMap<String, Vec<String>>,
}

/// `auth_token` 反序列化：接受单个字符串或字符串数组（`auth_token = "sk-..."`
/// 或 `auth_token = ["sk-a", "sk-b"]`）。
fn de_auth_token<'de, D>(de: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let v = serde_json::Value::deserialize(de)?;
    match v {
        serde_json::Value::String(s) => Ok(vec![s]),
        serde_json::Value::Array(items) => items
            .into_iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| D::Error::custom("auth_token 数组元素必须是字符串"))
            })
            .collect(),
        _ => Err(D::Error::custom("auth_token 必须是字符串或字符串数组")),
    }
}

/// TOML 原始形态的 provider 段：字段全 Option，preset 填充后再收敛成
/// 对外 `ProviderConfig`（至少一个 `*_base_url` 必填）。
#[derive(Deserialize)]
struct RawProviderConfig {
    /// 显式 preset 别名；缺省时「名称即 preset」。
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    base_url_anthropic: Option<String>,
    #[serde(default)]
    base_url_openai: Option<String>,
    #[serde(default)]
    api_key_env: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    model_map: HashMap<String, String>,
}

/// provider 惯例默认（spec §6 Provider 格局调研 + 2026-08-04/07 端点探测）。
/// 双协议 provider（anthropic + openai 端点都有）必须显式 `protocol`，不猜。
/// 默认 env 名均可被 `api_key_env` 覆盖。pub 暴露供 bot 侧 `/provider` 表单
/// 预填默认值（见 `src/provider.rs` 的 preset 规范化）。
pub struct ProviderPreset {
    pub name: &'static str,
    /// anthropic 协议端点；`None` = 该 preset 不提供 anthropic 端点。
    pub base_url_anthropic: Option<&'static str>,
    /// openai 协议端点；`None` = 该 preset 不提供 openai 端点。
    pub base_url_openai: Option<&'static str>,
    /// 默认 env 变量名（可被 `api_key_env` 覆盖）。
    pub api_key_env: &'static str,
}

const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "anthropic",
        base_url_anthropic: Some("https://api.anthropic.com"),
        base_url_openai: None,
        api_key_env: "ANTHROPIC_API_KEY",
    },
    ProviderPreset {
        name: "openai",
        base_url_anthropic: None,
        base_url_openai: Some("https://api.openai.com/v1"),
        api_key_env: "OPENAI_API_KEY",
    },
    ProviderPreset {
        name: "deepseek",
        base_url_anthropic: Some("https://api.deepseek.com/anthropic"),
        base_url_openai: Some("https://api.deepseek.com"),
        api_key_env: "DEEPSEEK_API_KEY",
    },
    ProviderPreset {
        name: "kimi",
        base_url_anthropic: Some("https://api.moonshot.cn/anthropic"),
        base_url_openai: Some("https://api.moonshot.cn/v1"),
        api_key_env: "MOONSHOT_API_KEY",
    },
    ProviderPreset {
        name: "glm",
        base_url_anthropic: Some("https://open.bigmodel.cn/api/anthropic"),
        base_url_openai: Some("https://open.bigmodel.cn/api/paas/v4"),
        api_key_env: "ZHIPU_API_KEY",
    },
    ProviderPreset {
        name: "minimax",
        base_url_anthropic: Some("https://api.minimaxi.com/anthropic"),
        base_url_openai: Some("https://api.minimaxi.com/v1"),
        api_key_env: "MINIMAX_API_KEY",
    },
    ProviderPreset {
        name: "ark",
        base_url_anthropic: Some("https://ark.cn-beijing.volces.com/api/plan"),
        base_url_openai: Some("https://ark.cn-beijing.volces.com/api/plan/v3"),
        api_key_env: "ARK_API_KEY",
    },
    ProviderPreset {
        name: "dashscope",
        base_url_anthropic: None,
        base_url_openai: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        api_key_env: "DASHSCOPE_API_KEY",
    },
    ProviderPreset {
        name: "gemini",
        base_url_anthropic: None,
        base_url_openai: Some("https://generativelanguage.googleapis.com/v1beta/openai"),
        api_key_env: "GEMINI_API_KEY",
    },
];

fn find_preset(name: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS.iter().find(|p| p.name == name)
}

/// 内置 provider preset 表（供 bot 侧表单选项与默认值预填使用）。
pub fn presets() -> &'static [ProviderPreset] {
    PROVIDER_PRESETS
}

/// raw → resolved：把每个 provider 收敛成对外 `ProviderConfig`。
/// - 名称即 preset（或显式 `preset = "..."` 别名）：preset 默认填入
///   `base_url_anthropic` / `base_url_openai`（各自独立，可同时存在），
///   显式字段覆盖对应协议位；
/// - 单协议 preset（缺某协议位 URL）→ 显式字段必须留空；显式填了对方
///   端点 → 配置错误（preset 不提供）；
/// - 无 preset（自定义 provider）→ 至少一个 `*_base_url` 必填。
fn resolve_providers(
    raw: HashMap<String, RawProviderConfig>,
) -> Result<HashMap<String, ProviderConfig>> {
    let mut out = HashMap::with_capacity(raw.len());
    for (name, r) in raw {
        let preset_name = r.preset.as_deref().unwrap_or(&name);
        let preset = find_preset(preset_name);
        let (base_url_anthropic, base_url_openai) = match preset {
            Some(p) => resolve_preset_urls(
                p,
                &name,
                preset_name,
                r.base_url_anthropic,
                r.base_url_openai,
            )?,
            None => {
                let a = r.base_url_anthropic.unwrap_or_default();
                let o = r.base_url_openai.unwrap_or_default();
                if a.is_empty() && o.is_empty() {
                    return Err(GatewayError::Config(format!(
                        "provider.{name}: 自定义 provider 至少需要 base_url_anthropic / base_url_openai 之一"
                    )));
                }
                (a, o)
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
                base_url_anthropic: option_string(base_url_anthropic),
                base_url_openai: option_string(base_url_openai),
                api_key_env,
                api_key: r.api_key,
                model_map: r.model_map,
            },
        );
    }
    Ok(out)
}

/// 空字符串归 None（非空为 Some）。
fn option_string(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

/// 按 preset 解析 `base_url_anthropic` / `base_url_openai`：preset 默认 +
/// 显式覆盖。preset 不提供的端点显式写了 → 错（避免误把错误的 URL 落到错
/// 的协议位）。
fn resolve_preset_urls(
    p: &ProviderPreset,
    name: &str,
    preset_name: &str,
    explicit_anthropic: Option<String>,
    explicit_openai: Option<String>,
) -> Result<(String, String)> {
    let anthropic = match explicit_anthropic {
        Some(b) => {
            if p.base_url_anthropic.is_none() {
                return Err(GatewayError::Config(format!(
                    "provider.{name}: preset '{preset_name}' 不提供 anthropic 端点，不能写 base_url_anthropic"
                )));
            }
            b
        }
        None => p.base_url_anthropic.unwrap_or_default().to_string(),
    };
    let openai = match explicit_openai {
        Some(b) => {
            if p.base_url_openai.is_none() {
                return Err(GatewayError::Config(format!(
                    "provider.{name}: preset '{preset_name}' 不提供 openai 端点，不能写 base_url_openai"
                )));
            }
            b
        }
        None => p.base_url_openai.unwrap_or_default().to_string(),
    };
    Ok((anthropic, openai))
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
                    provider_overlay: default_provider_overlay(),
                    default_provider: g.default_provider,
                    auth_token: g.auth_token,
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
                provider_overlay: default_provider_overlay(),
                default_provider: None,
                auth_token: Vec::new(),
                providers,
                routes: Vec::new(),
            },
        };
        cfg.apply_env_overrides();
        let mut cfg = cfg.with_expanded_paths();
        cfg.merge_provider_overlay()?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// env 覆盖（spec §6.3，优先级高于 TOML）。空值忽略，避免
    /// `SEBAS_GATEWAY_LISTEN=` 抹掉已配置的 listen。在 validate 之前运行。
    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("SEBAS_GATEWAY_LISTEN")
            && !v.is_empty()
        {
            self.listen = v;
        }
        if let Ok(v) = std::env::var("SEBAS_GATEWAY_PROVIDER_OVERLAY")
            && !v.is_empty()
        {
            self.provider_overlay = v;
        }
    }

    /// 合并 bot 侧 provider 变更文件（`/provider` 命令写入）。
    /// 文件缺失时是 no-op；格式错误/字段无效则启动即报错（fail fast）。
    fn merge_provider_overlay(&mut self) -> Result<()> {
        let path = std::path::Path::new(&self.provider_overlay);
        if !path.exists() {
            return Ok(());
        }
        let raw = std::fs::read_to_string(path).map_err(|e| {
            GatewayError::Config(format!(
                "读取 provider overlay {} 失败: {e}",
                path.display()
            ))
        })?;
        let file: ProviderOverlay = serde_json::from_str(&raw).map_err(|e| {
            GatewayError::Config(format!(
                "解析 provider overlay {} 失败: {e}",
                path.display()
            ))
        })?;
        for name in &file.deleted {
            self.providers.remove(name);
        }
        for (name, item) in file.providers {
            // overlay 条目与顶层 `[provider.*]` 同语义：preset / *_base_url /
            // api_key_env / api_key，交给同一个 raw→resolved 管线，支持「选
            // preset + 填密钥」的最小写法（地址由 preset 补全）。
            // 注：旧 overlay 里若残留 `protocol` 字段会被静默忽略——schema
            // 已切到 per-protocol base_url。
            let raw = RawProviderConfig {
                preset: item
                    .get("preset")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                base_url_anthropic: item
                    .get("base_url_anthropic")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                base_url_openai: item
                    .get("base_url_openai")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                api_key_env: item
                    .get("api_key_env")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                api_key: item
                    .get("api_key")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                model_map: HashMap::new(),
            };
            let mut resolved =
                resolve_providers(HashMap::from([(name.clone(), raw)])).map_err(|e| {
                    GatewayError::Config(format!("provider overlay 里 '{name}' 无效: {e}"))
                })?;
            let provider = resolved
                .remove(&name)
                .expect("resolve_providers keeps the input name");
            self.providers.insert(name, provider);
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.providers.is_empty() {
            return Err(GatewayError::Config("provider 不能为空".into()));
        }
        for (name, p) in &self.providers {
            if p.base_url_anthropic.is_none() && p.base_url_openai.is_none() {
                return Err(GatewayError::Config(format!(
                    "provider.{name}: base_url_anthropic / base_url_openai 至少需要一项"
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
        for (i, t) in self.auth_token.iter().enumerate() {
            if t.is_empty() {
                return Err(GatewayError::Config(format!(
                    "gateway.auth_token[{i}] 不能为空字符串"
                )));
            }
        }
        Ok(())
    }

    fn with_expanded_paths(mut self) -> Self {
        self.usage_file = expand_tilde(&self.usage_file);
        self.provider_overlay = expand_tilde(&self.provider_overlay);
        self
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
                    "gateway provider '{name}' 使用明文 api_key（config 内联或 /provider overlay 写入；如需更严格的密钥管理请改用 api_key_env）"
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
                base_url_anthropic: Some("gateway://self".to_string()),
                base_url_openai: Some("gateway://self".to_string()),
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

/// provider overlay 文件的 wire 结构（与 router::crud::FileStore 同一格式）。
#[derive(Deserialize)]
struct ProviderOverlay {
    #[serde(default)]
    providers: HashMap<String, serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    deleted: Vec<String>,
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

    /// 隔离 provider overlay 后解析：把 SEBAS_GATEWAY_PROVIDER_OVERLAY 指向
    /// 不存在的路径，避免测试读到开发机 ~/.sebas/providers.json 影响断言。
    /// 调用方必须已持有 LOCK（串行化所有 env 访问）。
    fn parse_isolated(raw: &str) -> Result<GatewayConfig> {
        unsafe {
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                "__sebas_test_no_overlay__.json",
            );
        }
        GatewayConfig::parse(raw)
    }

    const FULL_EXAMPLE: &str = r#"
[gateway]
default_provider = "anthropic"

auth_token = "sk-gw-local-dev"

[provider.anthropic]
api_key_env = "ANTHROPIC_API_KEY"

[provider.deepseek]
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
        let cfg = parse_isolated(FULL_EXAMPLE).expect("full example should parse");
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
        assert_eq!(cfg.auth_token, vec!["sk-gw-local-dev".to_string()]);
        assert_eq!(cfg.providers.len(), 2);
        let anth = cfg.providers.get("anthropic").expect("anthropic provider");
        assert_eq!(
            anth.base_url_anthropic.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert!(anth.base_url_openai.is_none());
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
        let cfg = parse_isolated(&raw).expect("should parse with [feishu] present");
        assert_eq!(cfg.providers.len(), 2);
    }

    #[test]
    fn missing_gateway_section_is_config_error() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let err = parse_isolated("[feishu]\napp_id = \"x\"\n").expect_err("missing [gateway]");
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
        let cfg = parse_isolated(FULL_EXAMPLE).expect("parse");
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
auth_token = "sk-test"
[provider.anthropic]
api_key = "test-key"
[gateway.routes]
"gpt-*" = ["openai"]
"#;
        let err = parse_isolated(raw).expect_err("unknown provider should error");
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
auth_token = "sk-test"
[provider.anthropic]
api_key = "test-key"
"#;
        let cfg = parse_isolated(raw).expect("parse");
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
auth_token = "sk-test"
# 名称即 preset：base_url_anthropic / api_key_env 自动按 preset 填
[provider.deepseek]
# 不写任何 *_base_url → preset 默认填双协议 URL

# 显式字段覆盖 preset 默认
[provider.openai]
base_url_openai = "http://localhost:9099/v1"
api_key_env = "MY_OPENAI_KEY"
"#;
        let cfg = parse_isolated(raw).expect("preset config should parse");

        let ds = cfg.providers.get("deepseek").expect("deepseek provider");
        assert_eq!(
            ds.base_url_anthropic.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(
            ds.base_url_openai.as_deref(),
            Some("https://api.deepseek.com")
        );
        assert_eq!(ds.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));

        let oai = cfg.providers.get("openai").expect("openai provider");
        // 单协议 preset（openai）：只填 base_url_openai，anthropic 缺位
        assert!(oai.base_url_anthropic.is_none());
        // 显式 base_url_openai 覆盖 preset 默认
        assert_eq!(
            oai.base_url_openai.as_deref(),
            Some("http://localhost:9099/v1")
        );
        assert_eq!(oai.api_key_env.as_deref(), Some("MY_OPENAI_KEY"));
    }

    #[test]
    fn preset_dual_protocol_fills_both_urls() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        // 双协议 preset 不再需要显式 protocol：两条 base_url 各自按 preset 填入，
        // 由请求方按协议选 URL（缺位 → ProtocolMismatch）。
        let raw = r#"
[gateway]
auth_token = "sk-test"
[provider.deepseek]
"#;
        let cfg = parse_isolated(raw).expect("dual-protocol preset without explicit url");
        let ds = cfg.providers.get("deepseek").expect("deepseek provider");
        assert_eq!(
            ds.base_url_anthropic.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(
            ds.base_url_openai.as_deref(),
            Some("https://api.deepseek.com")
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
auth_token = "sk-test"
[provider.my-openai]
preset = "openai"
"#;
        let cfg = parse_isolated(raw).expect("preset alias should parse");
        let p = cfg.providers.get("my-openai").expect("aliased provider");
        assert!(p.base_url_anthropic.is_none());
        assert_eq!(
            p.base_url_openai.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(p.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
    }

    #[test]
    fn preset_single_protocol_with_foreign_url_errors() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
auth_token = "sk-test"
[provider.openai]
base_url_anthropic = "https://anthropic-from-openai.example"
"#;
        let err = parse_isolated(raw).expect_err("openai preset with base_url_anthropic");
        let msg = err.to_string();
        assert!(
            msg.contains("openai") && msg.contains("anthropic"),
            "error should name preset and missing endpoint: {msg}"
        );
    }

    #[test]
    fn custom_provider_without_any_url_errors() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
auth_token = "sk-test"
[provider.my-custom]
api_key_env = "MY_KEY"
"#;
        let err = parse_isolated(raw).expect_err("custom provider without URLs");
        assert!(
            err.to_string().contains("base_url_anthropic")
                || err.to_string().contains("base_url_openai")
                || err.to_string().contains("provider.my-custom"),
            "custom provider must require at least one URL: {err}"
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
auth_token = "sk-test"
[provider.anthropic]
api_key = "test-key"
"#;
        let cfg = parse_isolated(raw).expect("preset + api_key should parse");
        let p = cfg.providers.get("anthropic").expect("anthropic provider");
        assert_eq!(
            p.base_url_anthropic.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert!(p.base_url_openai.is_none());
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

auth_token = "sk-test"
[provider.deepseek]
"#;
        let cfg = parse_isolated(raw).expect("top-level provider should parse");
        assert_eq!(cfg.default_provider.as_deref(), Some("deepseek"));
        assert_eq!(cfg.auth_token, vec!["sk-test".to_string()]);
        let ds = cfg.providers.get("deepseek").expect("deepseek provider");
        assert_eq!(
            ds.base_url_anthropic.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(
            ds.base_url_openai.as_deref(),
            Some("https://api.deepseek.com")
        );
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
        let cfg = parse_isolated(raw).expect("provider-only config should parse");
        assert_eq!(cfg.listen, "127.0.0.1:8787");
        assert_eq!(cfg.auth_token.len(), 0);
        assert_eq!(cfg.routes.len(), 0);
        let p = cfg.providers.get("anthropic").expect("anthropic provider");
        assert_eq!(
            p.base_url_anthropic.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert!(p.base_url_openai.is_none());
        assert_eq!(p.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn top_level_provider_table_with_explicit_url_and_plain_key() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        // 顶层 `[provider.*]`：preset 自动填充 + 显式 base_url_anthropic/api_key 覆盖。
        let raw = r#"
[provider.deepseek]

[provider.ark]
base_url_anthropic = "https://ark.cn-beijing.volces.com/api/plan"
api_key = "test-ark-key"
"#;
        let cfg = parse_isolated(raw).expect("top-level provider table should parse");
        let ds = cfg.providers.get("deepseek").expect("deepseek provider");
        assert_eq!(
            ds.base_url_anthropic.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(
            ds.base_url_openai.as_deref(),
            Some("https://api.deepseek.com")
        );
        assert_eq!(ds.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));

        let ark = cfg.providers.get("ark").expect("ark provider");
        assert_eq!(
            ark.base_url_anthropic.as_deref(),
            Some("https://ark.cn-beijing.volces.com/api/plan")
        );
        assert_eq!(ark.api_key.as_deref(), Some("test-ark-key"));
        assert_eq!(ark.api_key_env, None);
    }

    // -------------------- auth_token --------------------

    #[test]
    fn auth_token_accepts_single_string_or_array() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let single = r#"
[gateway]
auth_token = "sk-one"
[provider.anthropic]
api_key = "test-key"
"#;
        let cfg = parse_isolated(single).expect("single auth_token should parse");
        assert_eq!(cfg.auth_token, vec!["sk-one".to_string()]);

        let many = r#"
[gateway]
auth_token = ["sk-a", "sk-b"]
[provider.anthropic]
api_key = "test-key"
"#;
        let cfg = parse_isolated(many).expect("array auth_token should parse");
        assert_eq!(cfg.auth_token, vec!["sk-a".to_string(), "sk-b".to_string()]);
    }

    #[test]
    fn auth_token_empty_string_errors() {
        let _g = LOCK.lock().unwrap();
        // SAFETY: 本测试文件用 LOCK 串行化所有 env 访问（见 tests 模块注释）。
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
auth_token = ""
[provider.anthropic]
api_key = "test-key"
"#;
        let err = parse_isolated(raw).expect_err("empty auth_token must error");
        assert!(
            err.to_string().contains("auth_token"),
            "error should mention auth_token: {err}"
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
auth_token = "sk-test"
[provider.anthropic]
api_key = "test-key"
"#;
        let mut cfg = parse_isolated(raw).expect("parse");
        assert!(!cfg.debug);
        cfg.enable_debug_test_provider();

        assert!(cfg.debug);
        let test = cfg.providers.get("test").expect("test provider injected");
        assert_eq!(test.base_url_anthropic.as_deref(), Some("gateway://self"));
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
    fn overlay_merges_providers_and_deleted() {
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        std::fs::write(
            &overlay,
            r#"{
                "providers": {
                    "deepseek": { "preset": "deepseek", "protocol": "anthropic", "api_key": "sk-ds" },
                    "anthropic": { "api_key_env": "ANTHROPIC_API_KEY_V2" }
                },
                "deleted": ["openai"]
            }"#,
        )
        .unwrap();
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_PROVIDER_OVERLAY", overlay.to_str().unwrap());
        }

        // 新语法：顶层 [provider.*]；overlay 条目走同一个 preset 解析管线。
        let raw = r#"
[provider.anthropic]
[provider.openai]
"#;
        // 注意：不能用 parse_isolated——它会重置 overlay 环境变量；本测试
        // 自己设置了 overlay 文件，必须直接 parse。
        let cfg = GatewayConfig::parse(raw).expect("parse with overlay");

        assert!(
            !cfg.providers.contains_key("openai"),
            "tombstone must remove openai"
        );
        let ds = cfg
            .providers
            .get("deepseek")
            .expect("overlay added deepseek");
        assert_eq!(
            ds.base_url_anthropic.as_deref(),
            Some("https://api.deepseek.com/anthropic"),
            "preset must fill the anthropic endpoint from the hardcoded table"
        );
        assert_eq!(
            ds.base_url_openai.as_deref(),
            Some("https://api.deepseek.com"),
            "preset must fill the openai endpoint from the hardcoded table"
        );
        assert_eq!(
            ds.api_key.as_deref(),
            Some("sk-ds"),
            "overlay api_key must be consumed"
        );
        assert_eq!(
            ds.api_key_env, None,
            "explicit api_key must not get preset env"
        );
        let anth = cfg.providers.get("anthropic").expect("anthropic kept");
        assert_eq!(
            anth.base_url_anthropic.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert!(anth.base_url_openai.is_none());
        assert_eq!(anth.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY_V2"));
    }
}
