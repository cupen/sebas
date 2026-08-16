//! 模型能力参数 + provider 模型列表 → Claude Code 4 个 MODEL 环境变量的映射。
//!
//! 模型能力（context_window / max_output_tokens）每种模型是固定的，这里
//! 硬编码一份已知模型能力表（数据来源：models.dev / 各 provider 公开规格）。
//! 模型名可能是 provider 自定义名（如 `deepseek-v4-pro[1m]`），此时：
//! - `[n]` 后缀（`[128k]` / `[1m]`）既是模型名的一部分，也表示上下文长度，
//!   直接解析出来覆盖表里的 context_window；
//! - 表里认不出的模型回退默认值（`DEFAULT_CAPS`）。
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

/// 无 `[n]` 后缀、表里也匹配不到时的兜底能力。
const DEFAULT_CAPS: ModelCaps = ModelCaps {
    context_window: Some(128_000),
    max_output_tokens: Some(8192),
};

/// 硬编码模型能力表。键为模型名前缀（最长匹配优先）；值给 context 与
/// output 上限。`max_output_tokens: None` 意为「回退 DEFAULT_CAPS.output」。
#[rustfmt::skip]
const MODEL_CAPABILITIES: &[(&str, Option<u64>, Option<u64>)] = &[
    // ---- DeepSeek（官方为 128K 上下文；output 各版本不同）----
    ("deepseek-chat",       Some(131_072), Some(8192)),
    ("deepseek-reasoner",   Some(131_072), Some(8192)),
    ("deepseek-v3.1",       Some(131_072), Some(8192)),
    ("deepseek-v3.2",       Some(131_072), Some(32_768)),
    // ---- 通用 Anthropic 自家模型 ----
    ("claude-opus", Some(200_000), Some(32_768)),
    ("claude-sonnet", Some(200_000), Some(32_768)),
    ("claude-haiku", Some(200_000), Some(32_768)),
];

/// 解析模型名末尾的 `[n]` 后缀：`[128k]`→128_000、`[1m]`→1_000_000（十进制）。
/// 无后缀返回 `None`。求整用小写单位；`K`/`M` 大写也接受。
pub fn parse_context_suffix(name: &str) -> Option<u64> {
    let bytes = name.as_bytes();
    let end = bytes.len();
    // 从后往前找 `]`；必须在末尾且前面有 `[`。
    if end == 0 || bytes[end - 1] != b']' {
        return None;
    }
    let open = name.rfind('[')?;
    if open == 0 || open >= end - 1 {
        return None;
    }
    let inside = &name[open + 1..end - 1];
    let (digits, unit) = inside.split_at(inside.len().saturating_sub(1));
    if digits.is_empty() {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    match unit {
        "k" | "K" => Some(n * 1000),
        "m" | "M" => Some(n * 1_000_000),
        _ => None,
    }
}

/// 表里最长前缀匹配。
fn table_match(name: &str) -> Option<ModelCaps> {
    MODEL_CAPABILITIES
        .iter()
        .filter(|(prefix, _, _)| name.starts_with(prefix))
        .max_by_key(|(prefix, _, _)| prefix.len())
        .map(|(_, ctx, out)| ModelCaps {
            context_window: *ctx,
            max_output_tokens: *out,
        })
}

/// 解析一个模型名的完整能力：`[n]` 后缀优先（覆盖 context），其次查表，
/// 兜底 `DEFAULT_CAPS`。output 未在任一来源给出时用 `context / 4`。
pub fn resolve_caps(name: &str) -> ModelCaps {
    let suffix = parse_context_suffix(name);
    let table = table_match(name).unwrap_or(DEFAULT_CAPS);
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
            let last = models
                .last()
                .expect("non-empty slice has last")
                .clone();
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
    fn suffix_overrides_table_context() {
        // 同一前缀；[1m] 后缀覆盖表里的 131072
        let caps = resolve_caps("deepseek-v4-pro[1m]");
        assert_eq!(caps.context_window, Some(1_000_000));
        // output 走 DEFAULT_CAPS（表里匹配不到这个前缀）
        assert_eq!(caps.max_output_tokens, Some(8192));
    }

    #[test]
    fn table_match_fills_known_model() {
        let caps = resolve_caps("deepseek-chat");
        assert_eq!(caps.context_window, Some(131_072));
        assert_eq!(caps.max_output_tokens, Some(8192));
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let caps = resolve_caps("totally-unknown-xyz");
        assert_eq!(caps, DEFAULT_CAPS);
    }

    #[test]
    fn single_model_sets_all_vars() {
        let env = map_to_env(&["only-model".to_string()]);
        assert_eq!(env.model.as_deref(), Some("only-model"));
        assert_eq!(env.opus.as_deref(), Some("only-model"));
        assert_eq!(env.sonnet.as_deref(), Some("only-model"));
        assert_eq!(env.haiku.as_deref(), Some("only-model"));
    }

    #[test]
    fn multi_model_maps_strong_to_weak() {
        let env = map_to_env(&[
            "pro-max[1m]".to_string(),
            "pro".to_string(),
            "flash".to_string(),
        ]);
        assert_eq!(env.model.as_deref(), Some("pro-max[1m]"));
        assert_eq!(env.opus.as_deref(), Some("pro-max[1m]"));
        assert_eq!(env.sonnet.as_deref(), Some("pro"));
        assert_eq!(env.haiku.as_deref(), Some("flash"));
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