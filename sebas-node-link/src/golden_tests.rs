//! 黄金样本回归（type-session-vocabularies tasks 1.4 / 5.1）——链路侧。
//!
//! `golden_link_vocabulary.json` 是**重构前**的代码序列化出的真实 wire 载荷：
//! `SessionSummary`（phase 并集 × mode）与 `SessionOp::ApprovalAnswer`
//! （决定值的**裸字符串**形状）。类型化后必须逐字段不变。

use crate::{SessionOp, SessionSummary};
use serde_json::Value;

const GOLDEN: &str = include_str!("golden_link_vocabulary.json");

fn golden() -> Value {
    serde_json::from_str(GOLDEN).expect("golden file is valid JSON")
}

fn assert_round_trip<T>(sample: &Value, label: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let typed: T = serde_json::from_value(sample.clone())
        .unwrap_or_else(|e| panic!("{label} 反序列化失败: {e}\n样本: {sample}"));
    let back = serde_json::to_value(&typed).expect("re-serialize");
    assert_eq!(&back, sample, "{label} 往返后载荷形状改变");
}

#[test]
fn golden_session_summaries_round_trip() {
    let doc = golden();
    let summaries = doc["session_summaries"].as_array().expect("array");
    assert_eq!(summaries.len(), 8 + 3, "phase 并集 + 三种非 ask mode");
    for (i, sample) in summaries.iter().enumerate() {
        assert_round_trip::<SessionSummary>(sample, &format!("session_summary[{i}]"));
    }
}

#[test]
fn golden_approval_answers_round_trip() {
    let doc = golden();
    let answers = doc["approval_answers"].as_array().expect("array");
    assert_eq!(answers.len(), 3, "节点链路只承载三值（无 escalate）");
    for (i, sample) in answers.iter().enumerate() {
        assert_round_trip::<SessionOp>(sample, &format!("approval_answer[{i}]"));
        // 决定的线拼写是**裸字符串**，不是 `{"decision": ...}` 信封。
        assert!(
            sample["decision"].is_string(),
            "节点链路的决定必须是裸字符串: {sample}"
        );
    }
}
