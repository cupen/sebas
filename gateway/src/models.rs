//! 模型能力参数 + provider 模型列表 → Claude Code 4 个 MODEL 环境变量的映射。
//!
//! 每种模型的参数是固定的，只在这里定义**一份**：不同 provider 背后的服务
//! 商提供同名模型时参数一致，所以按模型名精确查找共享同一条目。
//!
//! 模型名可能是 provider 自定义名（如 `deepseek-v4-pro[1m]`），此时：
//! - `[n]` 后缀（`[128k]` / `[1m]`）既是模型名的一部分，也表示上下文长度，
//!   解析出来**覆盖**静态定义里的 context_window；
//! - 注册表里认不出的模型回退默认值（`DEFAULT_CAPS`）。
//!
//! 强→弱映射规则（Claude Code 假定 `OPUS` 最强、`SONNET` 次之、`HAIKU`
//! 最弱）：
//! - provider 只给 1 个模型 → 4 个 MODEL 变量全设该模型；
//! - 给 ≥2 个 → `ANTHROPIC_MODEL` + `ANTHROPIC_DEFAULT_OPUS_MODEL` = 最强
//!   （列表头），`ANTHROPIC_DEFAULT_SONNET_MODEL` = 次强（第 2 个），
//!   `ANTHROPIC_DEFAULT_HAIKU_MODEL` = 最弱（列表尾）。

/// 单个模型的上下文长度 / 输出上限。`Option` 表示未知，调用方按需回退。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCaps {
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

/// 无 `[n]` 后缀、注册表里也匹配不到时的兜底能力。
const DEFAULT_CAPS: ModelCaps = ModelCaps {
    context_window: Some(128_000),
    max_output_tokens: Some(8192),
};

/// 静态模型定义：每种模型写一份，同名模型跨 provider 共享。
#[derive(Debug, Clone, Copy)]
pub struct ModelDef {
    /// 规范模型名（提交的模型名与之精确匹配；`[n]` 后缀剥离后比对）。
    pub name: &'static str,
    pub context_window: u64,
    pub max_output_tokens: u64,
}

/// 静态模型注册表。参数来自公开规格（数据来源：models.dev / 各 provider 文档）。
/// 同名模型跨 provider 共享同一参数定义。
#[rustfmt::skip]
const MODELS: &[ModelDef] = &[
    // ---- DeepSeek V4 家族 ----
    ModelDef { name: "deepseek-v4-flash", context_window: 128_000,  max_output_tokens: 8192 },
    ModelDef { name: "deepseek-v4-pro",   context_window: 131_072,  max_output_tokens: 8192 },
    // ---- Anthropic 自家 ----
    ModelDef { name: "claude-opus",    context_window: 200_000, max_output_tokens: 32_768 },
    ModelDef { name: "claude-sonnet",  context_window: 200_000, max_output_tokens: 32_768 },
    ModelDef { name: "claude-haiku",   context_window: 200_000, max_output_tokens: 32_768 },
    // ---- OpenAI ----
    ModelDef { name: "gpt-4o",        context_window: 128_000, max_output_tokens: 16_384 },
    ModelDef { name: "gpt-4o-mini",   context_window: 128_000, max_output_tokens: 16_384 },
    ModelDef { name: "o1",            context_window: 128_000, max_output_tokens: 32_768 },
    ModelDef { name: "o1-pro",        context_window: 128_000, max_output_tokens: 100_000 },
    ModelDef { name: "o3",            context_window: 200_000, max_output_tokens: 100_000 },
    // ---- Kimi / Moonshot ----
    ModelDef { name: "moonshot-v1-8k",   context_window: 8_192,     max_output_tokens: 4096 },
    ModelDef { name: "moonshot-v1-32k",  context_window: 32_768,    max_output_tokens: 4096 },
    ModelDef { name: "moonshot-v1-128k", context_window: 128_000,   max_output_tokens: 4096 },
    ModelDef { name: "moonshot-v1-2m",   context_window: 2_000_000, max_output_tokens: 4096 },
    ModelDef { name: "kimi-k1.5",        context_window: 128_000,   max_output_tokens: 4096 },
    // ---- GLM (Zhipu) ----
    ModelDef { name: "glm-4",       context_window: 128_000, max_output_tokens: 4096 },
    ModelDef { name: "glm-4-plus",  context_window: 128_000, max_output_tokens: 8192 },
    ModelDef { name: "glm-4-air",   context_window: 128_000, max_output_tokens: 8192 },
    ModelDef { name: "glm-4-long",  context_window: 1_000_000, max_output_tokens: 1_000_000 },
    // ---- MiniMax ----
    ModelDef { name: "minmax-01",   context_window: 4_000_000, max_output_tokens: 4096 },
    // ---- Ark (ByteDance / Doubao) ----
    ModelDef { name: "doubao-1-5-pro-256k", context_window: 256_000, max_output_tokens: 4096 },
    ModelDef { name: "doubao-1-5-pro-128k", context_window: 128_000, max_output_tokens: 4096 },
    ModelDef { name: "doubao-1-5-pro-32k",  context_window: 32_768,  max_output_tokens: 8192 },
    ModelDef { name: "doubao-1-5-lite-32k", context_window: 32_768,  max_output_tokens: 4096 },
    ModelDef { name: "doubao-pro-256k",     context_window: 256_000, max_output_tokens: 4096 },
    ModelDef { name: "doubao-pro-128k",     context_window: 128_000, max_output_tokens: 4096 },
    ModelDef { name: "doubao-pro-32k",      context_window: 32_768,  max_output_tokens: 8192 },
    // ---- Dashscope (Qwen / Alibaba) ----
    ModelDef { name: "qwen-max",        context_window: 128_000, max_output_tokens: 8192 },
    ModelDef { name: "qwen2.5-72b",     context_window: 131_072, max_output_tokens: 8192 },
    ModelDef { name: "qwen2.5-turbo",   context_window: 1_000_000, max_output_tokens: 8192 },
    ModelDef { name: "qwen-max-long",   context_window: 2_000_000, max_output_tokens: 8192 },
    // ---- Gemini (Google) ----
    ModelDef { name: "gemini-2.0-flash", context_window: 1_048_576, max_output_tokens: 8192 },
    ModelDef { name: "gemini-2.0-pro",   context_window: 2_097_152, max_output_tokens: 8192 },
];

/// 注册表按规范名精确查找。
fn table_match(name: &str) -> Option<ModelCaps> {
    MODELS
        .iter()
        .find(|m| m.name == name)
        .map(|m| ModelCaps {
            context_window: Some(m.context_window),
            max_output_tokens: Some(m.max_output_tokens),
        })
}

/// 解析模型名末尾的 `[n]` 后缀：`[128k]`→128_000、`[1m]`→1_000_000（十进制）。
/// 无后缀返回 `None`。`K`/`M` 大写也接受。
pub fn parse_context_suffix(name: &str) -> Option<u64> {
    let bytes = name.as_bytes();
    let end = bytes.len();
    if end == 0 || bytes[end - 1] != b']' {
        return None;
    }
    let open = name.rfind('[')?;
    if open == 0 || open >= end - 1 {
        return None;
    }
    let inside = &name[open + 1..end - 1];
    let (digits, unit) = inside.split_at(inside.len().saturating_sub(1));
    let n: u64 = digits.parse().ok()?;
    match unit {
        "k" | "K" => Some(n * 1000),
        "m" | "M" => Some(n * 1_000_000),
        _ => None,
    }
}

/// 解析一个模型名的完整能力：
/// 1. 剥离 `[n]` 后缀得到基准名 + 可选解析出的上下文；
/// 2. 基准名查静态注册表 → context 与 output；
/// 3. 后缀存在 → 覆盖 context_window；output 未给出时用 context / 4 兜底；
/// 4. 注册表未命中 → `DEFAULT_CAPS`（后缀仍可覆盖 context）。
pub fn resolve_caps(name: &str) -> ModelCaps {
    let (base, suffix) = match parse_context_suffix(name) {
        Some(ctx) => (&name[..name.rfind('[').unwrap_or(name.len())], Some(ctx)),
        None => (name, None),
    };
    let table = table_match(base).unwrap_or(DEFAULT_CAPS);
    let context_window = suffix.or(table.context_window);
    let max_output_tokens = table
        .max_output_tokens
        .or_else(|| context_window.map(|c| (c / 4).max(1024)));
    ModelCaps {
        context_window,
        max_output_tokens,
    }
}

/// Claude Code 消费的 4 个 MODEL 环境变量的取值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeModelEnv {
    /// ANTHROPIC_MODEL
    pub model: Option<String>,
    /// ANTHROPIC_DEFAULT_OPUS_MODEL
    pub opus: Option<String>,
    /// ANTHROPIC_DEFAULT_SONNET_MODEL
    pub sonnet: Option<String>,
    /// ANTHROPIC_DEFAULT_HAIKU_MODEL
    pub haiku: Option<String>,
}

impl ClaudeModelEnv {
    /// 生成 4 个环境变量名→值 的映射（仅含 `Some` 项）。
    pub fn to_env_map(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        if let Some(m) = &self.model {
            v.push(("ANTHROPIC_MODEL", m.clone()));
        }
        if let Some(o) = &self.opus {
            v.push(("ANTHROPIC_DEFAULT_OPUS_MODEL", o.clone()));
        }
        if let Some(s) = &self.sonnet {
            v.push(("ANTHROPIC_DEFAULT_SONNET_MODEL", s.clone()));
        }
        if let Some(h) = &self.haiku {
            v.push(("ANTHROPIC_DEFAULT_HAIKU_MODEL", h.clone()));
        }
        v
    }
}

/// 按强→弱把 provider 的模型列表映射到 4 个 MODEL 变量。返回全 `None` 表示
/// provider 没配 `models`（调用方跳过 env 注入）。
pub fn map_to_env(models: &[String]) -> ClaudeModelEnv {
    match models {
        [] => ClaudeModelEnv::default(),
        [m] => ClaudeModelEnv {
            model: Some(m.clone()),
            opus: Some(m.clone()),
            sonnet: Some(m.clone()),
            haiku: Some(m.clone()),
        },
        [first, second, ..] => {
            let last = models.last().expect("non-empty slice has last").clone();
            ClaudeModelEnv {
                model: Some(first.clone()),
                opus: Some(first.clone()),
                sonnet: Some(second.clone()),
                haiku: Some(last),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_parses_k_and_m() {
        assert_eq!(parse_context_suffix("deepseek-v4-pro[1m]"), Some(1_000_000));
        assert_eq!(parse_context_suffix("x[128k]"), Some(128_000));
        assert_eq!(parse_context_suffix("x[32K]"), Some(32_000));
        assert!(parse_context_suffix("deepseek-v4-flash").is_none());
        assert!(parse_context_suffix("x").is_none());
        assert!(parse_context_suffix("x[abc]").is_none());
    }

    #[test]
    fn static_def_fills_known_model() {
        // 注册表里的规范名精确命中
        let caps = resolve_caps("deepseek-v4-flash");
        assert_eq!(caps.context_window, Some(128_000));
        assert_eq!(caps.max_output_tokens, Some(8192));
    }

    #[test]
    fn suffix_overrides_static_context() {
        // deepseek-v4-pro 静态定义 131072；[1m] 后缀覆盖成 1M
        let caps = resolve_caps("deepseek-v4-pro[1m]");
        assert_eq!(caps.context_window, Some(1_000_000));
        assert_eq!(caps.max_output_tokens, Some(8192));
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let caps = resolve_caps("totally-unknown-xyz");
        assert_eq!(caps, DEFAULT_CAPS);
    }

    #[test]
    fn known_model_with_unknown_suffix_keeps_table_output() {
        // claude-opus 有表；[99k] 后缀覆盖 context，output 保留表的
        let caps = resolve_caps("claude-opus[99k]");
        assert_eq!(caps.context_window, Some(99_000));
        assert_eq!(caps.max_output_tokens, Some(32_768));
    }

    #[test]
    fn single_model_sets_all_vars() {
        let env = map_to_env(&["deepseek-v4-flash".to_string()]);
        assert_eq!(env.model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(env.opus.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(env.sonnet.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(env.haiku.as_deref(), Some("deepseek-v4-flash"));
    }

    #[test]
    fn multi_model_maps_strong_to_weak() {
        let env = map_to_env(&[
            "deepseek-v4-pro[1m]".to_string(),
            "deepseek-v4-pro".to_string(),
            "deepseek-v4-flash".to_string(),
        ]);
        assert_eq!(env.model.as_deref(), Some("deepseek-v4-pro[1m]"));
        assert_eq!(env.opus.as_deref(), Some("deepseek-v4-pro[1m]"));
        assert_eq!(env.sonnet.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(env.haiku.as_deref(), Some("deepseek-v4-flash"));
    }

    #[test]
    fn two_models_second_is_sonnet_weakest_is_haiku() {
        let env = map_to_env(&["a".to_string(), "b".to_string()]);
        assert_eq!(env.opus.as_deref(), Some("a"));
        assert_eq!(env.sonnet.as_deref(), Some("b"));
        assert_eq!(env.haiku.as_deref(), Some("b"));
    }

    #[test]
    fn empty_models_yields_none() {
        let env = map_to_env(&[]);
        assert_eq!(env, ClaudeModelEnv::default());
        assert!(env.to_env_map().is_empty());
    }

    #[test]
    fn env_map_uses_expected_names() {
        let env = map_to_env(&["m".to_string()]);
        let vars = env.to_env_map();
        assert!(vars.contains(&("ANTHROPIC_MODEL", "m".to_string())));
        assert!(vars.contains(&("ANTHROPIC_DEFAULT_OPUS_MODEL", "m".to_string())));
        assert!(vars.contains(&("ANTHROPIC_DEFAULT_SONNET_MODEL", "m".to_string())));
        assert!(vars.contains(&("ANTHROPIC_DEFAULT_HAIKU_MODEL", "m".to_string())));
    }
}