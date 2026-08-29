//! gateway 配置模型与解析（见 openspec/specs/gateway-core/spec.md）。
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
use crate::key_resolver::{EnvKeyResolver, KeyResolver, hint_from_provider};
use crate::proto::WireProtocol;

/// 顶层包装：容忍同一 config.toml 中的 `[feishu]` / `[acp.*]` 等无关段，
/// 只取 `[gateway]`。有意不复用 root `Config::parse`——gateway 的运行边界
/// 与配置 schema 独立于 sebas 主进程（见 openspec/specs/gateway-core/spec.md）。
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
    /// 限流配置（`[gateway.rate_limit]`）。缺省不限流。
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub routes: Vec<RouteGroup>,
    /// 模型别名 → upstream model（`None` = 别名透传）。由 provider overlay
    /// 的 `model_aliases` 段编译而来；`RouteTable` 用它在非 namespace 路径
    /// 做 rename。对外结构体序列化用 `serde(skip)`——这是派生数据。
    #[serde(skip)]
    pub model_aliases: HashMap<String, Option<String>>,
    /// 本配置来自的 config.toml 路径（reload 用）。`#[serde(skip)]`：
    /// wire 上不存在；由调用方（gateway_cmd / admin reload）注入。
    /// 缺省读 `SEBAS_GATEWAY_CONFIG`，再退 `~/.sebas/config.toml`。
    #[serde(skip)]
    pub config_source: String,
}

/// `[gateway.rate_limit]`：token-bucket 限流。缺省不限流。
///
/// - `rpm`: 每分钟请求数（便捷写法，本实现忽略）
/// - `capacity` + `refill_per_sec`: token-bucket 原始参数：
///   容量 = 瞬时允许的突发请求数；refill_per_sec = 每秒补充速率。
/// 都未设（capacity=None 且 rpm=None）→ 不限流。
#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub struct RateLimitConfig {
    /// 每分钟请求数简便写法。仅占位：语义与 capacity/refill_per_sec 一致，
    /// 由 `RateLimiter` 统一解析；此处保留字段供配置书写。
    #[serde(default)]
    pub rpm: Option<u64>,
    /// token-bucket 容量（允许的最大突发）。
    #[serde(default)]
    pub capacity: Option<u64>,
    /// 每秒补充速率。
    #[serde(default = "default_refill_per_sec")]
    pub refill_per_sec: f64,
}

fn default_refill_per_sec() -> f64 {
    1.0
}

impl RateLimitConfig {
    /// 是否启用限流：capacity 或 rpm 任一设置即启用。
    pub fn enabled(&self) -> bool {
        self.capacity.is_some() || self.rpm.is_some()
    }

    /// 解析成 (capacity, refill_per_sec)。缺省均不限流时返回 None。
    /// `rpm` 优先：capacity=rpm、refill=rpm/60。
    pub fn bucket_params(&self) -> Option<(u64, f64)> {
        if let Some(rpm) = self.rpm {
            return Some((rpm, rpm as f64 / 60.0));
        }
        self.capacity.map(|cap| (cap, self.refill_per_sec))
    }
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

/// reload 用的 config.toml 来源：`SEBAS_GATEWAY_CONFIG` env，退
/// `~/.sebas/config.toml`。调用方（gateway_cmd）可在 parse 后覆盖。
fn default_config_source() -> String {
    std::env::var("SEBAS_GATEWAY_CONFIG")
        .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "~/.sebas/config.toml".into())
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
    /// 上游 model id 改名映射：`{旧名: 新名}`。gateway 收到旧名时改写成
    /// 新名再走路由；用于同一 provider 多名（同模型被官方重命名或别名）。
    /// 与 `models` 功能部分重合但独立——
    /// `models` 决定 OPUS/SONNET/HAIKU 赋值，`model_map` 决定请求时
    /// model 字段的最终名字。
    #[serde(default)]
    pub model_map: HashMap<String, String>,
    /// 按从强到弱排列的模型名列表（手写）。`[n]` 后缀（如 `[1m]`）既是
    /// 模型名一部分，也表示上下文长度。见 `crate::models::map_to_env`。
    /// `models` 顺序 = 强→弱，用于 Claude Code
    /// 4 个 MODEL 环境变量（OPUS/SONNET/HAIKU）的赋值；与 `model_map`
    /// 不重复，前者定 model 列表的强弱档位，后者定上游 id 重命名。
    #[serde(default)]
    pub models: Vec<String>,
}

/// 给定请求协议返回对应上游 URL；两项都为 None 视为未配置。
impl ProviderConfig {
    pub fn url_for(&self, proto: WireProtocol) -> Option<&str> {
        match proto {
            WireProtocol::Anthropic => self.base_url_anthropic.as_deref(),
            WireProtocol::OpenAi => self.base_url_openai.as_deref(),
        }
    }

    /// 把本 provider 手写的 `models`（从强到弱）映射成 Claude Code 的 4 个
    /// MODEL 环境变量。`models` 为空 → 全 `None`（调用方跳过注入，用系统默认）。
    pub fn model_env(&self) -> crate::models::ClaudeModelEnv {
        crate::models::map_to_env(&self.models)
    }

    /// 解析某个模型的静态能力（上下文 / 输出上限）。`[n]` 后缀自动覆盖上下文。
    pub fn model_caps(&self, model: &str) -> crate::models::ModelCaps {
        crate::models::resolve_caps(model)
    }

    /// 最强模型（列表头）＝ provider 的默认模型。空列表 → `None`。
    pub fn default_model(&self) -> Option<&str> {
        self.models.first().map(String::as_str)
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
    #[serde(default = "default_provider_overlay")]
    provider_overlay: String,
    #[serde(default)]
    default_provider: Option<String>,
    #[serde(default, deserialize_with = "de_auth_token")]
    auth_token: Vec<String>,
    #[serde(default)]
    rate_limit: RateLimitConfig,
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
    #[serde(default)]
    models: Vec<String>,
}

/// provider 惯例默认（见 openspec/specs/provider-management/spec.md 的 Provider 格局调研）。
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
    /// 该 provider 提供的 model 列表（静态约定，覆盖主页宣传的常用 model；
    /// 不调 /v1/models 动态拉取，详见 docs/... 或 bead sebas-63f.2 设计讨论）。
    pub models: &'static [&'static str],
}

const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "anthropic",
        base_url_anthropic: Some("https://api.anthropic.com"),
        base_url_openai: None,
        api_key_env: "ANTHROPIC_API_KEY",
        models: &[
            "claude-opus-4-20250514",
            "claude-sonnet-4-20250514",
            "claude-haiku-4-20250514",
            "claude-3-7-sonnet-20250219",
            "claude-3-5-haiku-20241022",
        ],
    },
    ProviderPreset {
        name: "openai",
        base_url_anthropic: None,
        base_url_openai: Some("https://api.openai.com/v1"),
        api_key_env: "OPENAI_API_KEY",
        models: &[
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4-turbo",
            "o1",
            "o1-mini",
            "o3-mini",
            "gpt-3.5-turbo",
        ],
    },
    ProviderPreset {
        name: "deepseek",
        base_url_anthropic: Some("https://api.deepseek.com/anthropic"),
        base_url_openai: Some("https://api.deepseek.com"),
        api_key_env: "DEEPSEEK_API_KEY",
        models: &["deepseek-chat", "deepseek-reasoner"],
    },
    ProviderPreset {
        name: "kimi",
        base_url_anthropic: Some("https://api.moonshot.cn/anthropic"),
        base_url_openai: Some("https://api.moonshot.cn/v1"),
        api_key_env: "MOONSHOT_API_KEY",
        models: &[
            "moonshot-v1-8k",
            "moonshot-v1-32k",
            "moonshot-v1-128k",
            "kimi-k2-0711-preview",
        ],
    },
    ProviderPreset {
        name: "glm",
        base_url_anthropic: Some("https://open.bigmodel.cn/api/anthropic"),
        base_url_openai: Some("https://open.bigmodel.cn/api/paas/v4"),
        api_key_env: "ZHIPU_API_KEY",
        models: &[
            "glm-4-plus",
            "glm-4-0520",
            "glm-4-air",
            "glm-4-airx",
            "glm-4-flash",
        ],
    },
    ProviderPreset {
        name: "minimax",
        base_url_anthropic: Some("https://api.minimaxi.com/anthropic"),
        base_url_openai: Some("https://api.minimaxi.com/v1"),
        api_key_env: "MINIMAX_API_KEY",
        // TODO: 实际 ids 待确认 (MiniMax 官方 model 命名常变)
        models: &[
            "MiniMax-Text-01",
            "MiniMax-VL-01",
            "abab6.5s-chat",
            "abab6.5g-chat",
        ],
    },
    ProviderPreset {
        name: "ark",
        base_url_anthropic: Some("https://ark.cn-beijing.volces.com/api/plan"),
        base_url_openai: Some("https://ark.cn-beijing.volces.com/api/plan/v3"),
        api_key_env: "ARK_API_KEY",
        // TODO: 实际 endpoint ids (doubao-pro / lite 等含版本号后缀) 待确认
        models: &["doubao-pro", "doubao-lite", "doubao-1-5-pro"],
    },
    ProviderPreset {
        name: "dashscope",
        base_url_anthropic: None,
        base_url_openai: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        api_key_env: "DASHSCOPE_API_KEY",
        models: &["qwen-turbo", "qwen-plus", "qwen-max", "qwen-long"],
    },
    ProviderPreset {
        name: "gemini",
        base_url_anthropic: None,
        base_url_openai: Some("https://generativelanguage.googleapis.com/v1beta/openai"),
        api_key_env: "GEMINI_API_KEY",
        models: &[
            "gemini-1.5-pro",
            "gemini-1.5-flash",
            "gemini-1.5-flash-8b",
            "gemini-2.0-flash-exp",
        ],
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
                models: r.models,
            },
        );
    }
    Ok(out)
}

/// 校验候选 provider 条目：overlay JSON Map 在内存里完整跑 resolve 管线
/// （preset 解析、URL 校验），不碰文件。admin 写路径（providers CRUD、
/// reload）用它做「写前 400」判定；返回解析后的 ProviderConfig 供进一步检查。
/// 错误信息恒含 provider 名（admin 400 body 直接可读）。
pub fn validate_provider_entry(
    name: &str,
    item: &serde_json::Map<String, serde_json::Value>,
) -> Result<ProviderConfig> {
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
        models: parse_models_list(item),
    };
    let mut resolved = resolve_providers(HashMap::from([(name.to_string(), raw)]))?;
    resolved
        .remove(name)
        .ok_or_else(|| GatewayError::Config(format!("provider.{name}: 解析结果丢失")))
}

/// 空字符串归 None（非空为 Some）。
fn option_string(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

/// 从 provider overlay JSON 条目里读 `models`。支持两种格式：
/// - 数组（gateway 直接读）：`["a", "b"]`
/// - 逗号分隔字符串（来自 `/provider` 表单提交）：`"a,b"`
/// 保持书写顺序 = 强→弱。缺省 → 空列表。
fn parse_models_list(item: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    match item.get("models") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(serde_json::Value::String(s)) => s
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
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
                    provider_overlay: g.provider_overlay,
                    default_provider: g.default_provider,
                    auth_token: g.auth_token,
                    rate_limit: g.rate_limit,
                    providers,
                    routes,
                    model_aliases: HashMap::new(),
                    config_source: default_config_source(),
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
                rate_limit: RateLimitConfig::default(),
                providers,
                routes: Vec::new(),
                model_aliases: HashMap::new(),
                config_source: default_config_source(),
            },
        };
        cfg.apply_env_overrides();
        let mut cfg = cfg.with_expanded_paths();
        cfg.merge_provider_overlay()?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// env 覆盖（优先级高于 TOML）。空值忽略，避免
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
            let provider = validate_provider_entry(&name, &item).map_err(|e| {
                GatewayError::Config(format!("provider overlay 里 '{name}' 无效: {e}"))
            })?;
            self.providers.insert(name, provider);
        }
        // 模型别名编译（D2）：每别名一条精确 RouteGroup 前置（胜过同名
        // config route）；rename 记入 cfg.model_aliases（RouteTable 在非
        // namespace 路径改写——namespace rest 不吃别名改写）。
        // 引用不存在 provider 的别名 drop + warn（外部写入的自愈，不 fail fast）。
        let mut alias_routes: Vec<RouteGroup> = Vec::new();
        for (alias, entry) in file.model_aliases {
            if !self.providers.contains_key(&entry.provider) {
                tracing::warn!(
                    alias = %alias,
                    provider = %entry.provider,
                    "model alias 引用不存在的 provider，已丢弃"
                );
                continue;
            }
            self.model_aliases
                .insert(alias.clone(), entry.upstream_model.clone());
            alias_routes.push(RouteGroup {
                model: alias,
                providers: vec![entry.provider],
            });
        }
        if !alias_routes.is_empty() {
            // 字典序稳定排列后整体前置到 config routes 之前。
            alias_routes.sort_by(|a, b| a.model.cmp(&b.model));
            alias_routes.append(&mut self.routes);
            self.routes = alias_routes;
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
        if let Some(rpm) = self.rate_limit.rpm
            && rpm == 0
        {
            return Err(GatewayError::Config(
                "gateway.rate_limit.rpm 必须 ≥ 1".into(),
            ));
        }
        if let Some(cap) = self.rate_limit.capacity
            && cap == 0
        {
            return Err(GatewayError::Config(
                "gateway.rate_limit.capacity 必须 ≥ 1".into(),
            ));
        }
        if self.rate_limit.enabled() && self.rate_limit.refill_per_sec <= 0.0 {
            return Err(GatewayError::Config(
                "gateway.rate_limit.refill_per_sec 必须 > 0".into(),
            ));
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
    ///
    /// 解析走 `KeyResolver` trait，默认 impl 是
    /// `EnvKeyResolver`（env → plain → none）。未来接 vault / 1Password /
    /// KMS 时可在 `build_state` 之前注入 `Arc<dyn KeyResolver>` 替代默认；
    /// 当前调用方（`server::build_state` / `debug::tests`）签名不变，
    /// 行为不变。
    pub fn resolve_api_keys(&self) -> Result<HashMap<String, String>> {
        self.resolve_api_keys_with(&EnvKeyResolver)
    }

    /// 同 `resolve_api_keys`，但接受注入的 resolver —— 给测试用 stub
    /// 或未来真接外部密钥后端时复用同一条路径。
    pub fn resolve_api_keys_with(
        &self,
        resolver: &dyn KeyResolver,
    ) -> Result<HashMap<String, String>> {
        let mut out = HashMap::with_capacity(self.providers.len());
        for (name, p) in &self.providers {
            // 内置 test provider（debug 模式注入）不触达外部上游，无需 key。
            if self.debug && name == "test" {
                continue;
            }
            let hint = hint_from_provider(p.api_key.as_deref(), p.api_key_env.as_deref());
            match resolver.resolve(&hint) {
                Ok(key) => {
                    out.insert(name.clone(), key);
                }
                Err(reason) => {
                    return Err(GatewayError::Config(format!("provider.{name}: {reason}")));
                }
            }
        }
        Ok(out)
    }
}

/// provider overlay 文件的 wire 结构（与 router::crud::FileStore 同一格式）。
#[derive(Deserialize)]
struct ProviderOverlay {
    #[serde(default)]
    providers: HashMap<String, serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    deleted: Vec<String>,
    /// 模型别名：`alias -> { provider, upstream_model? }`。由 admin API /
    /// 手工编辑写入；引用不存在 provider 的别名在合并期 drop + warn。
    #[serde(default)]
    model_aliases: HashMap<String, ModelAliasEntry>,
}

/// providers.json `model_aliases` 段的单个别名 wire。
#[derive(Debug, Clone, Deserialize)]
struct ModelAliasEntry {
    provider: String,
    /// 缺省 = 别名即 upstream model（透传）。
    #[serde(default)]
    upstream_model: Option<String>,
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

    // 所有 config 测试都经此锁串行：env 覆盖测试会 set/remove
    // `SEBAS_GATEWAY_LISTEN`，单进程内并行跑会与其他调用 parse 的测试竞争。
    // 跨模块共享锁（crate::test_util::CONFIG_ENV_LOCK 的别名）——debug.rs 与本
    // 模块的测试都动 SEBAS_GATEWAY_LISTEN，必须互斥。
    static LOCK: &Mutex<()> = &crate::test_util::CONFIG_ENV_LOCK;

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
    fn provider_models_map_to_env_and_caps() {
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
default_provider = "deepseek"

[provider.deepseek]
models = ["deepseek-v4-pro[1m]", "deepseek-v4-flash"]
"#;
        let cfg = parse_isolated(raw).expect("provider with models should parse");
        let ds = cfg.providers.get("deepseek").expect("deepseek provider");
        // models 手写列表按从强到弱保留
        assert_eq!(ds.models, vec!["deepseek-v4-pro[1m]", "deepseek-v4-flash"]);
        // 最强 = 默认
        assert_eq!(ds.default_model(), Some("deepseek-v4-pro[1m]"));
        // env 映射：MODEL=OPUS=最强、SONNET=次强、HAIKU=最弱
        let env = ds.model_env();
        assert_eq!(env.model.as_deref(), Some("deepseek-v4-pro[1m]"));
        assert_eq!(env.opus.as_deref(), Some("deepseek-v4-pro[1m]"));
        assert_eq!(env.sonnet.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(env.haiku.as_deref(), Some("deepseek-v4-flash"));
        // 能力：无名 model 的 [1m] 后缀推导上下文，flash 查静态注册表
        let pro = ds.model_caps("deepseek-v4-pro[1m]");
        assert_eq!(pro.context_window, Some(1_000_000));
        let flash = ds.model_caps("deepseek-v4-flash");
        assert_eq!(flash.context_window, Some(1_000_000));
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

    // -------------------- rate_limit --------------------

    #[test]
    fn rate_limit_defaults_to_disabled() {
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let cfg = parse_isolated(FULL_EXAMPLE).expect("parse");
        assert!(!cfg.rate_limit.enabled(), "缺省必须不限流");
        assert_eq!(cfg.rate_limit.bucket_params(), None);
    }

    #[test]
    fn rate_limit_rpm_parses_and_derives_bucket() {
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
auth_token = "sk-test"
[gateway.rate_limit]
rpm = 60
[provider.anthropic]
api_key = "test-key"
"#;
        let cfg = parse_isolated(raw).expect("parse");
        assert!(cfg.rate_limit.enabled());
        let (cap, refill) = cfg.rate_limit.bucket_params().expect("bucket params");
        assert_eq!(cap, 60);
        assert!(
            (refill - 1.0).abs() < 1e-9,
            "rpm/60 = 1 token/s, got {refill}"
        );
    }

    #[test]
    fn rate_limit_capacity_and_refill_parse() {
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
auth_token = "sk-test"
[gateway.rate_limit]
capacity = 100
refill_per_sec = 5.0
[provider.anthropic]
api_key = "test-key"
"#;
        let cfg = parse_isolated(raw).expect("parse");
        let (cap, refill) = cfg.rate_limit.bucket_params().expect("bucket params");
        assert_eq!(cap, 100);
        assert_eq!(refill, 5.0);
    }

    #[test]
    fn rate_limit_rpm_zero_errors() {
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
auth_token = "sk-test"
[gateway.rate_limit]
rpm = 0
[provider.anthropic]
api_key = "test-key"
"#;
        let err = parse_isolated(raw).expect_err("rpm=0 must error");
        assert!(
            err.to_string().contains("rpm"),
            "error should mention rpm: {err}"
        );
    }

    #[test]
    fn rate_limit_capacity_zero_errors() {
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
auth_token = "sk-test"
[gateway.rate_limit]
capacity = 0
[provider.anthropic]
api_key = "test-key"
"#;
        let err = parse_isolated(raw).expect_err("capacity=0 must error");
        assert!(
            err.to_string().contains("capacity"),
            "error should mention capacity: {err}"
        );
    }

    #[test]
    fn rate_limit_zero_refill_errors() {
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
auth_token = "sk-test"
[gateway.rate_limit]
capacity = 10
refill_per_sec = 0.0
[provider.anthropic]
api_key = "test-key"
"#;
        let err = parse_isolated(raw).expect_err("refill=0 must error");
        assert!(
            err.to_string().contains("refill_per_sec"),
            "error should mention refill_per_sec: {err}"
        );
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

    /// overlay 带 model_aliases：编译为前置精确 RouteGroup + model_map 插入。
    /// 用 RouteTable::from_config + resolve 验证完整路由语义。
    #[test]
    fn overlay_model_aliases_compile_into_routes_and_model_map() {
        use crate::routing::RouteTable;
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
                    "beta": { "preset": "deepseek", "api_key": "sk-b" }
                },
                "model_aliases": {
                    "my-claude": { "provider": "beta", "upstream_model": "deepseek-chat" },
                    "bare": { "provider": "beta" }
                }
            }"#,
        )
        .unwrap();
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_PROVIDER_OVERLAY", overlay.to_str().unwrap());
        }
        let raw = r#"
[provider.anthropic]
[provider.openai]
"#;
        let cfg = GatewayConfig::parse(raw).expect("parse with alias overlay");

        // 带 upstream_model：resolve 后改写为 upstream；缺省：别名透传。
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(Some("my-claude"), crate::proto::WireProtocol::Anthropic)
            .expect("alias resolves");
        assert_eq!(d.provider, "beta");
        assert_eq!(d.upstream_model.as_deref(), Some("deepseek-chat"));

        let d = table
            .resolve(Some("bare"), crate::proto::WireProtocol::Anthropic)
            .expect("bare alias resolves");
        assert_eq!(d.provider, "beta");
        assert_eq!(d.upstream_model.as_deref(), Some("bare"), "缺省 upstream 透传别名");

        // 别名 RouteGroup 存在且排在 config routes 之前。
        assert!(
            cfg.routes
                .iter()
                .position(|r| r.model == "my-claude")
                .is_some_and(|i| cfg.routes.iter().take(i).all(|r| r.model != "m*")),
            "alias groups precede config routes"
        );
    }

    /// 别名胜过同名 config route（alias 组前置 = 顺序扫描先命中）。
    #[test]
    fn overlay_alias_beats_same_named_config_route() {
        use crate::routing::RouteTable;
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        std::fs::write(
            &overlay,
            r#"{
                "providers": { "beta": { "preset": "deepseek", "api_key": "sk-b" } },
                "model_aliases": { "m1": { "provider": "beta" } }
            }"#,
        )
        .unwrap();
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_PROVIDER_OVERLAY", overlay.to_str().unwrap());
        }
        let raw = r#"
[provider.anthropic]
[provider.openai]
[gateway.routes]
m1 = ["anthropic"]
"#;
        let cfg = GatewayConfig::parse(raw).expect("parse");
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(Some("m1"), crate::proto::WireProtocol::Anthropic)
            .expect("m1 resolves");
        assert_eq!(d.provider, "beta", "alias must beat same-named config route");
    }

    /// 命名空间仍优先于别名：`beta/m1` 走 beta 的 rest 而非 alias 改写。
    #[test]
    fn overlay_namespace_still_beats_alias() {
        use crate::routing::RouteTable;
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        std::fs::write(
            &overlay,
            r#"{
                "providers": { "beta": { "preset": "deepseek", "api_key": "sk-b" } },
                "model_aliases": { "m1": { "provider": "beta", "upstream_model": "renamed" } }
            }"#,
        )
        .unwrap();
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_PROVIDER_OVERLAY", overlay.to_str().unwrap());
        }
        let raw = r#"
[provider.anthropic]
"#;
        let cfg = GatewayConfig::parse(raw).expect("parse");
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(Some("beta/m1"), crate::proto::WireProtocol::Anthropic)
            .expect("namespace resolves");
        assert_eq!(d.provider, "beta");
        assert_eq!(
            d.upstream_model.as_deref(),
            Some("m1"),
            "namespace rest 不吃 alias 的 model_map 改写"
        );
    }

    /// 引用不存在 provider 的别名 drop + warn，不启动失败。
    #[test]
    fn overlay_alias_to_missing_provider_dropped() {
        use crate::routing::RouteTable;
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        std::fs::write(
            &overlay,
            r#"{
                "providers": {},
                "model_aliases": { "ghost": { "provider": "nonexistent" } }
            }"#,
        )
        .unwrap();
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_PROVIDER_OVERLAY", overlay.to_str().unwrap());
        }
        let raw = r#"
[provider.anthropic]
"#;
        let cfg = GatewayConfig::parse(raw).expect("坏别名不导致启动失败");
        assert!(
            cfg.routes.iter().all(|r| r.model != "ghost"),
            "坏别名不得编译进 routes"
        );
        let table = RouteTable::from_config(&cfg);
        // ghost 落到 anthropic（唯一 provider 隐式默认）而非 502。
        let d = table
            .resolve(Some("ghost"), crate::proto::WireProtocol::Anthropic)
            .expect("fallback default");
        assert_eq!(d.provider, "anthropic");
    }

    /// 校验辅助：无效候选（无 preset 无 URL）Err 且错误信息含 provider 名；
    /// 有效 preset 候选解析出 URL。
    #[test]
    fn validate_provider_entry_rejects_invalid_and_names_provider() {
        let mut bad = serde_json::Map::new();
        bad.insert("name".into(), serde_json::json!("mystery"));
        let err = validate_provider_entry("mystery", &bad).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("mystery"), "错误信息须含 provider 名: {msg}");
        assert!(
            msg.contains("base_url"),
            "错误信息须说明缺 URL: {msg}"
        );

        let mut good = serde_json::Map::new();
        good.insert("preset".into(), serde_json::json!("deepseek"));
        let cfg = validate_provider_entry("deepseek", &good).expect("preset 候选有效");
        assert!(cfg.base_url_openai.is_some(), "preset 补全 URL");
    }
}
