//! 节点域：执行节点的管理面视图（add-domain-layer 4.1）。
//!
//! 原「同一形状两处声明」的收敛：core 通道 `NodeLinkOp::ListNodes` 的
//! `NodeView`（根 crate protocol.rs）与 webui `session_backend` 的
//! `NodeInfo` 是同一个概念的逐字段重声明。合并为本定义；`local` 保留为
//! 独立字段——本机节点不经过注册表（它是隐式的），但工作台必须能显示它。
//!
//! **wire 兼容**（spec「Wire and on-disk compatibility preserved」）：
//! `local` 是 `skip_serializing_if = false` 的可选字段——core 通道上的
//! `NodeView` JSON 与合并前**逐字节一致**（合并前它根本没有 local 键）；
//! webui 侧远端节点行同样省略 false 键，本机行由 `/api/nodes` 的显式
//! `json!` 字面量携带 `"local": true`（形状不变）。

use serde::{Deserialize, Serialize};

/// 节点在管理面上的视图（不含凭据哈希等敏感字段）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeView {
    /// 稳定节点标识。
    pub id: String,
    /// `online` / `offline` / `revoked`。
    pub status: String,
    /// 最后一次成功握手时间（unix 秒）；本机节点为 `None`（无握手概念）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_unix: Option<i64>,
    /// 首次配对时间（unix 秒）；本机节点为 0。
    pub created_unix: i64,
    /// 是否为主控本机（隐式节点，永远在线——否则你读不到这个响应）。
    /// 只在 `true` 时上 wire；本机条目由工作台在列首显式补入。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 形状钉：`local: false` 不上 wire——与合并前 core 通道 `NodeView`
    /// 的序列化形状逐字节一致（add-domain-layer 4.1）。
    #[test]
    fn remote_node_wire_shape_has_no_local_key() {
        let n = NodeView {
            id: "node-a".into(),
            status: "online".into(),
            last_seen_unix: Some(1_700_000_000),
            created_unix: 1_600_000_000,
            local: false,
        };
        assert_eq!(
            serde_json::to_value(&n).unwrap(),
            serde_json::json!({
                "id": "node-a",
                "status": "online",
                "last_seen_unix": 1_700_000_000,
                "created_unix": 1_600_000_000,
            })
        );
    }

    #[test]
    fn local_flag_serializes_when_true_and_defaults_on_missing() {
        let mut n = NodeView {
            id: "local".into(),
            status: "online".into(),
            last_seen_unix: None,
            created_unix: 0,
            local: true,
        };
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["local"], true);
        assert!(v.get("last_seen_unix").is_none());
        // 旧报文无 local 键 → false（webui 行形状的反序列化兼容）。
        let legacy: NodeView =
            serde_json::from_str(r#"{"id":"x","status":"offline","created_unix":1}"#).unwrap();
        assert!(!legacy.local);
        n.local = false;
        assert!(!serde_json::to_value(&n).unwrap().as_object().unwrap().contains_key("local"));
    }
}
