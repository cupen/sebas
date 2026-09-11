//! 模型能力参数 + provider 模型列表 -> Claude Code 4 个 MODEL 环境变量的映射。
//!
//! 每种模型的参数是固定的，只在这里定义**一份**：不同 provider 背后的服务
//! 商提供同名模型时参数一致，所以按模型名精确查找共享同一条目。
//!
//! 模型名可能是 provider 自定义名（如 `deepseek-v4-pro[1m]`），此时：
//! - `[n]` 后缀（`[128k]` / `[1m]`）既是模型名的一部分，也表示上下文长度，
//!   解析出来**覆盖**静态定义里的 context_window；
//! - 注册表里认不出的模型回退默认值（`DEFAULT_CAPS`）。
//!
//! 强->弱映射规则（Claude Code 假定 `OPUS` 最强、`SONNET` 次之、`HAIKU`
//! 最弱）：
//! - provider 只给 1 个模型 -> 4 个 MODEL 变量全设该模型；
//! - 给 >=2 个 -> `ANTHROPIC_MODEL` + `ANTHROPIC_DEFAULT_OPUS_MODEL` = 最强
//!   （列表头），`ANTHROPIC_DEFAULT_SONNET_MODEL` = 次强（第 2 个），
//!   `ANTHROPIC_DEFAULT_HAIKU_MODEL` = 最弱（列表尾）。

use serde::{Deserialize, Serialize};
use std::str::FromStr;

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

/// 静态模型注册表。参数来自 models.dev（https://github.com/anomalyco/models.dev）。
//
// last synced: 2026-08-17 from models.dev (manual paste)
//
// 这是手工粘贴的静态数据，每次更新都靠手抄——commit log 看起来像是同步机制，
// 实际不是。如需自动同步请改成 vendored JSON 或 xtask 拉取。
#[rustfmt::skip]
const MODELS: &[ModelDef] = &[
    // ---- DeepSeek V4 ----
    ModelDef { name: "deepseek-v4-flash", context_window: 1_000_000, max_output_tokens: 384_000 },
    ModelDef { name: "deepseek-v4-pro",   context_window: 1_000_000, max_output_tokens: 384_000 },
    // ---- Anthropic Claude ----
    ModelDef { name: "claude-sonnet", context_window: 200_000, max_output_tokens: 64_000 },
    ModelDef { name: "claude-opus",   context_window: 200_000, max_output_tokens: 32_000 },
    ModelDef { name: "claude-haiku",  context_window: 200_000, max_output_tokens: 64_000 },
    // ---- OpenAI ----
    ModelDef { name: "gpt-4o",        context_window: 128_000,   max_output_tokens: 16_384 },
    ModelDef { name: "gpt-4o-mini",   context_window: 128_000,   max_output_tokens: 16_384 },
    ModelDef { name: "o3",            context_window: 200_000,   max_output_tokens: 100_000 },
    ModelDef { name: "o4-mini",       context_window: 200_000,   max_output_tokens: 100_000 },
    ModelDef { name: "gpt-5.5-pro",   context_window: 1_050_000, max_output_tokens: 128_000 },
    // ---- Google Gemini ----
    ModelDef { name: "gemini-2.0-flash", context_window: 1_048_576, max_output_tokens: 8_192 },
    ModelDef { name: "gemini-2.5-pro",   context_window: 1_048_576, max_output_tokens: 65_536 },
    // ---- Alibaba Qwen ----
    ModelDef { name: "qwen-max", context_window: 32_768, max_output_tokens: 8_192 },
    // ---- Zhipu GLM ----
    ModelDef { name: "glm-4.5",  context_window: 131_072, max_output_tokens: 98_304 },
    ModelDef { name: "glm-4.6",  context_window: 204_800, max_output_tokens: 131_072 },
    ModelDef { name: "glm-4.7",  context_window: 204_800, max_output_tokens: 131_072 },
    ModelDef { name: "glm-5",    context_window: 204_800, max_output_tokens: 131_072 },
    ModelDef { name: "glm-5.2",  context_window: 1_000_000, max_output_tokens: 131_072 },
    // ---- Moonshot Kimi ----
    ModelDef { name: "kimi-k2-thinking", context_window: 262_144,   max_output_tokens: 262_144 },
    ModelDef { name: "kimi-k2.5",        context_window: 262_144,   max_output_tokens: 262_144 },
    ModelDef { name: "kimi-k3",          context_window: 1_048_576, max_output_tokens: 131_072 },
    // ---- MiniMax ----
    ModelDef { name: "minimax-m2.5", context_window: 204_800, max_output_tokens: 131_072 },
    ModelDef { name: "minimax-m3",   context_window: 512_000, max_output_tokens: 128_000 },
    // ---- ByteDance Seed ----
    ModelDef { name: "seed-1.6",      context_window: 256_000, max_output_tokens: 64_000 },
    ModelDef { name: "seed-2.0-pro",  context_window: 256_000, max_output_tokens: 128_000 },
    ModelDef { name: "seed-2.1-turbo", context_window: 256_000, max_output_tokens: 256_000 },
];

/// 注册表按规范名精确查找。
fn table_match(name: &str) -> Option<ModelCaps> {
    MODELS.iter().find(|m| m.name == name).map(|m| ModelCaps {
        context_window: Some(m.context_window),
        max_output_tokens: Some(m.max_output_tokens),
    })
}

/// 解析模型名末尾的 `[n]` 后缀：`[128k]`->128_000、`[1m]`->1_000_000（十进制）。
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
/// 2. 基准名查静态注册表 -> context 与 output；
/// 3. 后缀存在 -> 覆盖 context_window；output 未给出时用 context / 4 兜底；
/// 4. 注册表未命中 -> `DEFAULT_CAPS`（后缀仍可覆盖 context）。
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

/// 模型条目的能力标记词表（redesign-provider-models-settings D2）。
///
/// `text` 隐含于每个条目、**永不落盘**；`vision` / `audio` / `video` 显式
/// 存储，标注该模型接受图片 / 音频 / 视频输入。标记是元数据：不参与路由、
/// 协议选择或请求准入（spec「tags do not gate requests」）。
///
/// 未知标记一律**拒绝**（反序列化报错）——与仓库「不静默吞错」的 typed
/// rejection 姿态一致；见 `ModelCapability::from_str` 与 `ModelEntry` 的
/// Deserialize。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModelCapability {
    Vision,
    Audio,
    Video,
}

impl ModelCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            ModelCapability::Vision => "vision",
            ModelCapability::Audio => "audio",
            ModelCapability::Video => "video",
        }
    }
}

impl std::fmt::Display for ModelCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// wire 词 → 标记。未知词（含 `text`——它隐含、不落盘）→ Err。
impl std::str::FromStr for ModelCapability {
    type Err = UnknownCapability;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "vision" => Ok(ModelCapability::Vision),
            "audio" => Ok(ModelCapability::Audio),
            "video" => Ok(ModelCapability::Video),
            _ => Err(UnknownCapability(s.to_string())),
        }
    }
}

impl Serialize for ModelCapability {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

/// 未知能力标记的显式错误（task 1.4：拒绝语义的载体，错误信息含原词）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCapability(pub String);

impl std::fmt::Display for UnknownCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "未知能力标记 '{}'（词表：vision / audio / video；text 隐含不写）",
            self.0
        )
    }
}

impl std::error::Error for UnknownCapability {}

/// provider 模型列表的一个条目：模型 id + 该模型的能力标记。
///
/// 兼容读取（task 1.1 / design D1）：裸字符串反序列化为无显式标记的条目
/// （text 隐含），旧字符串列表数据零迁移继续可读；**每次写出一律是条目
/// 对象** `{"id": …, "tags": […]}`（tags 去重排序，`text` 永不出现）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    pub id: String,
    /// 显式多模态标记（升序去重）。`text` 隐含，不在此列。
    pub tags: Vec<ModelCapability>,
}

impl ModelEntry {
    /// 纯文本条目（无显式标记）。
    pub fn text_only(id: impl Into<String>) -> Self {
        ModelEntry {
            id: id.into(),
            tags: Vec::new(),
        }
    }

    /// 规范化标记：去重 + 升序（BTree 顺序 = 定义序），保证写回形状稳定。
    fn normalize_tags(mut tags: Vec<ModelCapability>) -> Vec<ModelCapability> {
        tags.sort();
        tags.dedup();
        tags
    }
}

impl<'de> Deserialize<'de> for ModelEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let v = serde_json::Value::deserialize(deserializer)?;
        match v {
            // 遗留形态：裸字符串 = 仅 text 的条目。
            serde_json::Value::String(s) => {
                if s.trim().is_empty() {
                    return Err(D::Error::custom("模型条目 id 不能为空字符串"));
                }
                Ok(ModelEntry::text_only(s))
            }
            serde_json::Value::Object(map) => {
                let id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| D::Error::custom("模型条目对象缺少非空 id 字段"))?;
                let tags = match map.get("tags") {
                    None | Some(serde_json::Value::Null) => Vec::new(),
                    Some(serde_json::Value::Array(arr)) => {
                        let mut out = Vec::with_capacity(arr.len());
                        for t in arr {
                            let word = t.as_str().ok_or_else(|| {
                                D::Error::custom("模型条目 tags 元素必须是字符串")
                            })?;
                            out.push(ModelCapability::from_str(word).map_err(|e| {
                                D::Error::custom(e.to_string())
                            })?);
                        }
                        ModelEntry::normalize_tags(out)
                    }
                    Some(_) => {
                        return Err(D::Error::custom("模型条目 tags 必须是字符串数组"));
                    }
                };
                // 显式写 "text" 的条目直接拒绝（text 隐含、永不入库）。
                if map.contains_key("text") {
                    return Err(D::Error::custom(
                        "模型条目不接受 text 字段（text 隐含于每个条目）",
                    ));
                }
                Ok(ModelEntry {
                    id: id.to_string(),
                    tags,
                })
            }
            _ => Err(D::Error::custom(
                "模型条目必须是字符串或 {\"id\", \"tags\"} 对象",
            )),
        }
    }
}

impl Serialize for ModelEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap as _;
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("tags", &self.tags)?;
        map.end()
    }
}

/// Claude Code 消费的 4 个 MODEL 环境变量的取值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeModelEnv {
    pub model: Option<String>,
    pub opus: Option<String>,
    pub sonnet: Option<String>,
    pub haiku: Option<String>,
}

impl ClaudeModelEnv {
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

/// 按强->弱把 provider 的模型条目映射到 4 个 MODEL 变量（env 映射按条目
/// **id** 解析，task 1.3——能力标记不影响 env 赋值）。返回全 `None` 表示
/// provider 没配 `models`（调用方跳过 env 注入）。
pub fn map_to_env(models: &[ModelEntry]) -> ClaudeModelEnv {
    match models {
        [] => ClaudeModelEnv::default(),
        [m] => ClaudeModelEnv {
            model: Some(m.id.clone()),
            opus: Some(m.id.clone()),
            sonnet: Some(m.id.clone()),
            haiku: Some(m.id.clone()),
        },
        [first, second, ..] => {
            let last = models.last().expect("non-empty slice has last").clone();
            ClaudeModelEnv {
                model: Some(first.id.clone()),
                opus: Some(first.id.clone()),
                sonnet: Some(second.id.clone()),
                haiku: Some(last.id),
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
        let caps = resolve_caps("deepseek-v4-flash");
        assert_eq!(caps.context_window, Some(1_000_000));
        assert_eq!(caps.max_output_tokens, Some(384_000));
    }

    #[test]
    fn suffix_overrides_static_context() {
        let caps = resolve_caps("deepseek-v4-pro[1m]");
        assert_eq!(caps.context_window, Some(1_000_000));
        assert_eq!(caps.max_output_tokens, Some(384_000));
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let caps = resolve_caps("totally-unknown-xyz");
        assert_eq!(caps, DEFAULT_CAPS);
    }

    #[test]
    fn known_model_with_unknown_suffix_keeps_table_output() {
        let caps = resolve_caps("claude-sonnet[99k]");
        assert_eq!(caps.context_window, Some(99_000));
        assert_eq!(caps.max_output_tokens, Some(64_000));
    }

    #[test]
    fn single_model_sets_all_vars() {
        let env = map_to_env(&[ModelEntry::text_only("deepseek-v4-flash")]);
        assert_eq!(env.model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(env.opus.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(env.sonnet.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(env.haiku.as_deref(), Some("deepseek-v4-flash"));
    }

    #[test]
    fn multi_model_maps_strong_to_weak() {
        let env = map_to_env(&[
            ModelEntry::text_only("deepseek-v4-pro[1m]"),
            ModelEntry::text_only("deepseek-v4-pro"),
            ModelEntry::text_only("deepseek-v4-flash"),
        ]);
        assert_eq!(env.model.as_deref(), Some("deepseek-v4-pro[1m]"));
        assert_eq!(env.opus.as_deref(), Some("deepseek-v4-pro[1m]"));
        assert_eq!(env.sonnet.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(env.haiku.as_deref(), Some("deepseek-v4-flash"));
    }

    #[test]
    fn capability_tags_do_not_affect_env_mapping() {
        // task 1.3：env 映射按条目 id 解析——标记是纯元数据。
        let tagged = ModelEntry {
            id: "vision-pro".into(),
            tags: vec![ModelCapability::Video, ModelCapability::Vision],
        };
        let plain = ModelEntry::text_only("vision-pro");
        assert_eq!(map_to_env(&[tagged]), map_to_env(&[plain]));
    }

    #[test]
    fn two_models_second_is_sonnet_weakest_is_haiku() {
        let env = map_to_env(&[ModelEntry::text_only("a"), ModelEntry::text_only("b")]);
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
        let env = map_to_env(&[ModelEntry::text_only("m")]);
        let vars = env.to_env_map();
        assert!(vars.contains(&("ANTHROPIC_MODEL", "m".to_string())));
        assert!(vars.contains(&("ANTHROPIC_DEFAULT_OPUS_MODEL", "m".to_string())));
        assert!(vars.contains(&("ANTHROPIC_DEFAULT_SONNET_MODEL", "m".to_string())));
        assert!(vars.contains(&("ANTHROPIC_DEFAULT_HAIKU_MODEL", "m".to_string())));
    }

    // -------------------- ModelEntry 形状与兼容（task 1.1 / 1.4）--------------------

    #[test]
    fn legacy_bare_string_reads_as_text_only_entry() {
        let e: ModelEntry = serde_json::from_str("\"deepseek-chat\"").expect("legacy string");
        assert_eq!(e, ModelEntry::text_only("deepseek-chat"));
        assert!(e.tags.is_empty(), "text 隐含：无显式标记");
    }

    #[test]
    fn entry_serializes_as_object_with_tags_never_text() {
        let e = ModelEntry {
            id: "m-1".into(),
            tags: vec![ModelCapability::Vision],
        };
        let v = serde_json::to_value(&e).expect("serialize");
        assert_eq!(v, serde_json::json!({"id": "m-1", "tags": ["vision"]}));
        let plain = serde_json::to_value(ModelEntry::text_only("m-2")).expect("serialize");
        assert_eq!(
            plain,
            serde_json::json!({"id": "m-2", "tags": []}),
            "写出一律是条目对象；text 永不出现"
        );
    }

    #[test]
    fn entry_round_trips_through_object_form() {
        let e: ModelEntry =
            serde_json::from_str(r#"{"id":"m","tags":["video","vision","vision"]}"#).expect("obj");
        // 去重 + 升序（定义序 vision < audio < video 派生自 enum 声明序）。
        assert_eq!(e.id, "m");
        assert_eq!(e.tags, vec![ModelCapability::Vision, ModelCapability::Video]);
    }

    #[test]
    fn unknown_capability_tag_is_rejected() {
        // task 1.4：未知标记按「拒绝」处理（typed error，绝不静默忽略）。
        let err = serde_json::from_str::<ModelEntry>(r#"{"id":"m","tags":["telepathy"]}"#)
            .expect_err("unknown tag must be rejected");
        assert!(err.to_string().contains("telepathy"), "{err}");
        // text 显式写出同样拒绝（隐含、永不落盘）。
        let err = serde_json::from_str::<ModelEntry>(r#"{"id":"m","tags":["text"]}"#)
            .expect_err("text must never be stored");
        assert!(err.to_string().contains("text"), "{err}");
    }

    #[test]
    fn entry_object_requires_non_empty_id() {
        assert!(serde_json::from_str::<ModelEntry>(r#"{"tags":[]}"#).is_err());
        assert!(serde_json::from_str::<ModelEntry>(r#"{"id":""}"#).is_err());
        assert!(serde_json::from_str::<ModelEntry>(r#"42"#).is_err());
    }
}
