//! 黄金样本回归（type-session-vocabularies tasks 1.4 / 4.3 / 5.1）。
//!
//! `golden_session_vocabulary.json` 是**重构前**的代码序列化出来的真实 wire
//! 载荷。本模块把它反序列化回结构体、再序列化一次，断言与样本逐字段一致：
//! 词汇类型化不得改变任何既有载荷的形状与拼写。
//!
//! 本模块只经过 serde（不直接引用词汇类型），因此在类型化前后都必须通过——
//! 它正是「零变化基线」的机械闸门。

use crate::session::{PermissionDecision, RemoteSessionView, TurnEntry};
use serde_json::Value;

const GOLDEN: &str = include_str!("golden_session_vocabulary.json");

fn golden() -> Value {
    serde_json::from_str(GOLDEN).expect("golden file is valid JSON")
}

/// 结构体往返：样本 → 结构体 → JSON，载荷必须逐字段等于样本。
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
fn golden_turn_entries_round_trip() {
    let doc = golden();
    let entries = doc["turn_entries"].as_array().expect("turn_entries array");
    assert_eq!(entries.len(), 7, "六种 element_type + prompt 条目");
    for (i, sample) in entries.iter().enumerate() {
        assert_round_trip::<TurnEntry>(sample, &format!("turn_entry[{i}]"));
    }
}

#[test]
fn golden_remote_session_views_round_trip() {
    let doc = golden();
    let views = doc["remote_session_views"]
        .as_array()
        .expect("remote_session_views array");
    assert_eq!(views.len(), 4, "四种 mode");
    for (i, sample) in views.iter().enumerate() {
        assert_round_trip::<RemoteSessionView>(sample, &format!("remote_session_view[{i}]"));
    }
}

#[test]
fn golden_permission_decisions_round_trip() {
    let doc = golden();
    let decisions = doc["permission_decisions"]
        .as_array()
        .expect("permission_decisions array");
    assert_eq!(decisions.len(), 4, "四值决策集");
    for (i, sample) in decisions.iter().enumerate() {
        assert_round_trip::<PermissionDecision>(sample, &format!("permission_decision[{i}]"));
    }
}

/// （tasks 5.1）**逐字节**线快照：每个拼写标量的序列化载荷必须恰好是
/// `"<拼写>"`——裸字符串、无信封、大小写与下划线逐字（`DONE` / `CrossMark` /
/// `permission_mode_result` 都是契约的一部分）。未知取值也必须原样回吐。
#[test]
fn golden_spelling_serialization_is_byte_identical() {
    use crate::session::{CardPhase, SessionMode, SessionPhase, TurnElementType, TurnKind};
    let doc = golden();

    fn assert_scalar_bytes<T: serde::Serialize>(spelling: &str, typed: T, label: &str) {
        let bytes = serde_json::to_string(&typed).expect("serialize");
        assert_eq!(
            bytes,
            format!("\"{spelling}\""),
            "{label} 的线载荷不是裸拼写（逐字节比对）"
        );
    }

    let lists: [(&str, fn(&str) -> String, &str); 5] = [
        (
            "session_phase_spellings",
            (|s| serde_json::to_string(&SessionPhase::from_wire(s)).unwrap()) as fn(&str) -> String,
            "SessionPhase",
        ),
        (
            "card_phase_spellings",
            |s| serde_json::to_string(&CardPhase::from_wire(s)).unwrap(),
            "CardPhase",
        ),
        (
            "session_mode_spellings",
            |s| serde_json::to_string(&SessionMode::from_wire(s)).unwrap(),
            "SessionMode",
        ),
        (
            "turn_kind_spellings",
            |s| serde_json::to_string(&TurnKind::from_wire(s)).unwrap(),
            "TurnKind",
        ),
        (
            "element_type_spellings",
            |s| serde_json::to_string(&TurnElementType::from_wire(s)).unwrap(),
            "TurnElementType",
        ),
    ];
    for (key, render, label) in lists {
        for spelling in doc[key].as_array().expect("spelling list") {
            let spelling = spelling.as_str().expect("spelling is a string");
            assert_eq!(
                render(spelling),
                format!("\"{spelling}\""),
                "{label} 的线载荷不是裸拼写（逐字节比对）"
            );
        }
    }
    // 未知取值原样回吐，不得被改写成某个已知拼写。
    assert_scalar_bytes(
        "brand_new",
        SessionPhase::from_wire("brand_new"),
        "unknown SessionPhase",
    );
    assert_scalar_bytes(
        "brand_new",
        TurnElementType::from_wire("brand_new"),
        "unknown TurnElementType",
    );

    // 决定是**标签信封**（与裸拼写词汇不同）：逐字节钉住信封与字段顺序。
    assert_eq!(
        serde_json::to_string(&PermissionDecision::AllowOnce).unwrap(),
        r#"{"decision":"allow_once"}"#
    );
    assert_eq!(
        serde_json::to_string(&PermissionDecision::AllowSession).unwrap(),
        r#"{"decision":"allow_session"}"#
    );
    assert_eq!(
        serde_json::to_string(&PermissionDecision::Deny).unwrap(),
        r#"{"decision":"deny"}"#
    );
    assert_eq!(
        serde_json::to_string(&PermissionDecision::Escalate {
            reason: "why".into()
        })
        .unwrap(),
        r#"{"decision":"escalate","reason":"why"}"#
    );
    assert_eq!(
        serde_json::to_string(&PermissionDecision::Unknown("yolo".into())).unwrap(),
        r#"{"decision":"yolo"}"#
    );
}

/// 词汇拼写清单本身也要被类型化的 `as_str()` 逐字覆盖（tasks 2.1 / 4.1）。
#[test]
fn golden_spelling_lists_match_typed_vocabularies() {
    use crate::session::{CardPhase, SessionPhase};
    let doc = golden();

    let phases: Vec<String> = doc["session_phase_spellings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    for spelling in &phases {
        assert_eq!(
            SessionPhase::from_wire(spelling).as_str(),
            spelling,
            "phase 拼写必须逐字保留"
        );
    }

    let cards: Vec<String> = doc["card_phase_spellings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    for spelling in &cards {
        assert_eq!(
            CardPhase::from_wire(spelling).as_str(),
            spelling,
            "卡相位拼写必须逐字保留"
        );
    }
}

