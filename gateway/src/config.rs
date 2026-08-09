use serde::Deserialize;
use std::collections::HashMap;

use crate::error::{GatewayError, Result};
use crate::proto::Protocol;

/// 顶层包装：容忍同一 config.toml 中的 `[feishu]` / `[acp.*]` 等无关段，
/// 只取 `[gateway]`。有意不复用 root `Config::parse`——gateway 的运行边界
/// 与配置 schema 独立于 sebas 主进程（spec §3）。
#[derive(Deserialize)]
struct GatewayFile {
    #[serde(default)]
    gateway: Option<GatewayConfig>,
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
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub keys: Vec<KeyConfig>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub routes: Vec<RouteRule>,
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
/// 令牌（非上游 provider 密钥），非空且全局不重复。
#[derive(Debug, Clone, Deserialize)]
pub struct KeyConfig {
    pub key: String,
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

/// 路由规则：按序匹配 model（glob），命中后转发到 `provider`。
#[derive(Debug, Clone, Deserialize)]
pub struct RouteRule {
    pub model: String,
    pub provider: String,
}

impl GatewayConfig {
    /// 解析顺序对齐 root house style（src/config.rs）：
    /// toml → env 覆盖（`SEBAS_GATEWAY_LISTEN`）→ validate → tilde 展开（`usage_file`）。
    pub fn parse(raw: &str) -> Result<Self> {
        let file: GatewayFile =
            toml::from_str(raw).map_err(|e| GatewayError::Config(format!("toml parse: {e}")))?;
        let mut cfg = file
            .gateway
            .ok_or_else(|| GatewayError::Config("config 缺少 [gateway] 段".into()))?;
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
            return Err(GatewayError::Config("gateway.providers 不能为空".into()));
        }
        for (name, p) in &self.providers {
            if p.base_url.is_empty() {
                return Err(GatewayError::Config(format!(
                    "gateway.providers.{name}.base_url 不能为空"
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
            if !self.providers.contains_key(&r.provider) {
                return Err(GatewayError::Config(format!(
                    "gateway.routes 引用了未定义的 provider '{}'",
                    r.provider
                )));
            }
        }
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (i, k) in self.keys.iter().enumerate() {
            if k.key.is_empty() {
                return Err(GatewayError::Config(format!(
                    "gateway.keys[{i}].key 不能为空"
                )));
            }
            if let Some(&first) = seen.get(k.key.as_str()) {
                return Err(GatewayError::Config(format!(
                    "gateway.keys[{i}] 与 keys[{first}] 的 key 重复"
                )));
            }
            seen.insert(k.key.as_str(), i);
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

    /// 解析每个 provider 的上游 api key：
    /// - `api_key_env` 指向的 env 变量必须存在且非空（错误信息只含变量名，绝不含 key 值）；
    /// - 否则回退明文 `api_key`（仅测试用，emit warn）；
    /// - 两者都缺 → Config 错误。
    pub fn resolve_api_keys(&self) -> Result<HashMap<String, String>> {
        let mut out = HashMap::with_capacity(self.providers.len());
        for (name, p) in &self.providers {
            let key = if let Some(env_var) = &p.api_key_env {
                match std::env::var(env_var) {
                    Ok(v) if !v.is_empty() => v,
                    _ => {
                        return Err(GatewayError::Config(format!(
                            "gateway.providers.{name}.api_key_env 指向的环境变量 '{env_var}' 未设置或为空"
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
                    "gateway.providers.{name} 未配置 api_key_env 或 api_key"
                )));
            };
            out.insert(name.clone(), key);
        }
        Ok(out)
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

[gateway.providers.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"

[gateway.providers.deepseek]
protocol = "anthropic"
base_url = "https://api.deepseek.com/anthropic"
api_key_env = "DEEPSEEK_API_KEY"

[[gateway.routes]]
model = "claude-*"
provider = "anthropic"

[[gateway.routes]]
model = "deepseek-*"
provider = "deepseek"
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
        let expected_suffix =
            std::path::Path::new(".sebas").join("gateway-usage.jsonl");
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
        assert_eq!(cfg.routes[0].provider, "anthropic");
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
[gateway.providers.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
[[gateway.routes]]
model = "gpt-*"
provider = "openai"
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
[gateway.providers.anthropic]
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
}
