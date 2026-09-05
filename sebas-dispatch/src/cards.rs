//! 卡片渲染配置的中立镜像（decouple-feishu-channel task 3）。
//!
//! `sebas-dispatch` 不再依赖 `sebas-feishu`，但 router 仍然需要
//! `CardConfig`（累积规则：截断/折叠/思考显示 在 router 侧解释，
//! `[card]` 渲染 knob 的最终解释权在 feishu adapter 侧）。
//!
//! 本镜像与 feishu crate 的 `CardConfig` 字段逐一相同（含
//! `deny_unknown_fields` 严格解析语义、缺省值、JSON 段名 `theme_color` /
//! `max_user_text_chars` / `max_tool_output_chars` / `fold_long_output` /
//! `thinking`），保证：
//! - `settings.json`（TOML `[card]` 首次落盘的全量快照）在两个 crate 间
//!   往返一致；
//! - core bin 装配时把 feishu adapter 持有的 `CardConfig` 按字段拷成镜像
//!   （`serde_json` 往返即可，两侧形状相同），`/settings` 读写闭环不被
//!   破坏。
//!
//! 将来第二个 adapter 有独立渲染配置段时，只需扩展本镜像 + core 转换，
//! 不需要动 feishu crate。

use serde::{Deserialize, Serialize};

/// 卡片流配置（原 `[card]` TOML 段，见 openspec/specs/feishu-cards/spec.md）。
/// 由 feishu adapter 持有的渲染配置拷贝而来；router 用它解释累积规则。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CardConfig {
    #[serde(default = "default_theme_color")]
    pub theme_color: String,
    /// 单元素文本软上限：超过则截断 + 追加灰注（(已折叠 N 字)）。
    #[serde(default = "default_max_user_text")]
    pub max_user_text_chars: usize,
    /// tool result 软上限：0（默认）= 完全不输出 tool call 的结果内容；
    /// \>0 时结果收进工具折叠面板，超过该值则折叠，完整内容保留。
    /// 代码另有 10240 硬上限兜底，配置无法放宽。
    #[serde(default = "default_max_tool_output")]
    pub max_tool_output_chars: usize,
    /// true（默认）：tool call 折叠成 collapsible_panel（默认收起），
    /// tool result 按 max_tool_output_chars 屏蔽/收纳；
    /// false：不折叠，全文内联展示（结果仍受 10240 硬上限约束）。
    #[serde(default = "default_true")]
    pub fold_long_output: bool,
    #[serde(default)]
    pub thinking: ThinkingDisplay,
}

/// 如何显示模型的 thinking 内容。`disable` 有意不暴露——留给未来同时关掉
/// agent 侧 thinking token 的特性。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingDisplay {
    /// 折叠进 collapsible_panel（默认）。
    #[default]
    Show,
    /// 彻底丢弃 ThinkingDelta（模型仍产生，只是不上卡）。
    Hide,
}

impl Default for CardConfig {
    fn default() -> Self {
        Self {
            theme_color: default_theme_color(),
            max_user_text_chars: default_max_user_text(),
            max_tool_output_chars: default_max_tool_output(),
            fold_long_output: default_true(),
            thinking: ThinkingDisplay::default(),
        }
    }
}

fn default_theme_color() -> String {
    "blue".into()
}
fn default_max_user_text() -> usize {
    4000
}
fn default_max_tool_output() -> usize {
    0
}
fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_feishu_contract() {
        let c = CardConfig::default();
        assert_eq!(c.theme_color, "blue");
        assert_eq!(c.max_user_text_chars, 4000);
        assert_eq!(c.max_tool_output_chars, 0);
        assert!(c.fold_long_output);
        assert_eq!(c.thinking, ThinkingDisplay::Show);
    }

    #[test]
    fn unknown_key_is_rejected() {
        let res: Result<CardConfig, _> = serde_json::from_str(r#"{"theme_colr": "blue"}"#);
        assert!(res.is_err(), "deny_unknown_fields 必须拒绝未知键");
    }

    #[test]
    fn serialized_keys_match_feishu_shape() {
        // JSON 段名必须与 feishu `CardConfig` 一致（settings.json 跨 crate
        // 往返依赖这一点）。
        let c = CardConfig {
            theme_color: "orange".into(),
            max_user_text_chars: 100,
            max_tool_output_chars: 50,
            fold_long_output: false,
            thinking: ThinkingDisplay::Hide,
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["theme_color"], "orange");
        assert_eq!(v["max_user_text_chars"], 100);
        assert_eq!(v["max_tool_output_chars"], 50);
        assert_eq!(v["fold_long_output"], false);
        assert_eq!(v["thinking"], "hide");
    }
}