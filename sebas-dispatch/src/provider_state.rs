//! `/provider` 运行态持久化：mode + default_selection。
//!
//! 这部分数据是 `state_store::PersistedState` 的一个子集视图。本模块保留
//! `ProviderRuntimeState` 类型与 `load()` / `update()` 自由函数 API（向后
//! 兼容），底层**全部委托给 `state_store`**（状态库是唯一权威，
//! retire-legacy-state-json 3.2/3.4：`state.json` 与 `SEBAS_STATE_FILE`
//! 都已退休，本模块不再有任何文件路径）。

use crate::state_store::{self, DefaultSelection, PersistedState};
use serde::{Deserialize, Serialize};

// Provider 路由模式词表已迁往 `sebas_domain::provider`（add-domain-layer
// 3.3，design D5「仍是词表的部分」），原位再导出保持既有路径可解析。
pub use sebas_domain::provider::ProviderMode;

/// 运行时持久化状态（mode + default_selection 的子集视图）。
///
/// 这是 `state_store::PersistedState` 的轻量投影 —— 只保留 spawn 翻译
/// 关心的两个字段。新代码建议直接用 `PersistedState`。
///
/// 设计要点：
/// - 字段都 `#[serde(default)]`：旧文件缺字段时仍能加载（向前兼容）。
/// - 整结构 `Default`：第一次跑没有 state.json 时 `load()` 直接返回这个。
/// - `default_selection` 镜像 `PersistedState::default_selection`（openspec/specs/provider-management/spec.md
///   合并 provider + model 到一个字段；不再有独立的 `default_provider_for_direct`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRuntimeState {
    #[serde(default)]
    pub mode: ProviderMode,
    #[serde(default)]
    pub default_selection: Option<DefaultSelection>,
}

impl From<&PersistedState> for ProviderRuntimeState {
    fn from(s: &PersistedState) -> Self {
        Self {
            mode: s.mode.clone(),
            default_selection: s.default_selection.clone(),
        }
    }
}

impl ProviderRuntimeState {
    /// 把当前 runtime state 应用到 `PersistedState`（其他字段保留）。
    pub fn apply_to(&self, s: &mut PersistedState) {
        s.mode = self.mode.clone();
        s.default_selection = self.default_selection.clone();
    }
}

/// 读当前运行态。底层走状态库（`state_store::load`）；库不可用时按
/// `Default::default()` 呈现并 warn 点名成因 —— runtime 状态不应让 sebas
/// 启动失败，但也**不**回退读任何文件。
pub fn load() -> ProviderRuntimeState {
    ProviderRuntimeState::from(&state_store::load())
}

/// 写入状态库（`state_store::save`，单事务提交）。
///
/// **重要**：这个 save 会把当前 PersistedState 整体覆盖（包括 providers +
/// deleted 字段）。调用方应该先 load → 改 → save，或者用 `update()` 闭包。
/// 状态库不可用时返回 Err（绝不写文件、也不静默成功）。
pub fn save(s: &ProviderRuntimeState) -> anyhow::Result<()> {
    let mut current = state_store::load();
    s.apply_to(&mut current);
    state_store::save(&current)
}

/// 读 → 改 → 写一气呵成。`update` 闭包基于当前 runtime state 做条件决策。
///
/// 返回落库后的最新 runtime state。状态库不可用时 Err。
pub fn update<F>(f: F) -> anyhow::Result<ProviderRuntimeState>
where
    F: FnOnce(&mut ProviderRuntimeState),
{
    let after = state_store::update(|persisted| {
        let mut rs = ProviderRuntimeState::from(&*persisted);
        f(&mut rs);
        rs.apply_to(persisted);
    })?;
    Ok(ProviderRuntimeState::from(&after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_off_with_no_default_provider() {
        let s = ProviderRuntimeState::default();
        assert_eq!(s.mode, ProviderMode::Off);
        assert_eq!(s.default_selection, None);
    }

    #[test]
    fn provider_mode_round_trips_all_three_variants() {
        for mode in [
            ProviderMode::Off,
            ProviderMode::Direct {
                provider: "deepseek".into(),
            },
            ProviderMode::Router,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let back: ProviderMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, back, "round-trip failed for {mode:?}");
        }
    }

    #[test]
    fn provider_mode_serializes_with_kind_tag() {
        // 关键不变量：snake_case tag 让 JSON 是稳定字符串而非 enum index。
        assert_eq!(
            serde_json::to_value(ProviderMode::Off).unwrap(),
            serde_json::json!({"kind": "off"})
        );
        assert_eq!(
            serde_json::to_value(ProviderMode::Direct {
                provider: "x".into()
            })
            .unwrap(),
            serde_json::json!({"kind": "direct", "provider": "x"})
        );
        assert_eq!(
            serde_json::to_value(ProviderMode::Router).unwrap(),
            serde_json::json!({"kind": "router"})
        );
    }

    /// save → load 往返保留全部字段（经状态库）。
    #[test]
    fn save_then_load_round_trips() {
        let _engine = crate::test_engine::install_fresh();
        let original = ProviderRuntimeState {
            mode: ProviderMode::Direct {
                provider: "anthropic".into(),
            },
            default_selection: Some(DefaultSelection::with_model("deepseek", "deepseek-chat")),
        };
        save(&original).unwrap();
        assert_eq!(load(), original);
    }

    /// `update()` 读 → 改 → 写都做完了，且返回值就是改后的状态。
    #[test]
    fn update_mutates_and_persists() {
        let _engine = crate::test_engine::install_fresh();
        let updated = update(|s| {
            s.mode = ProviderMode::Router;
            s.default_selection = Some(DefaultSelection::new("openai"));
        })
        .unwrap();
        assert_eq!(updated.mode, ProviderMode::Router);
        assert_eq!(
            updated
                .default_selection
                .as_ref()
                .map(|d| d.provider.as_str()),
            Some("openai")
        );
        assert_eq!(load(), updated);
    }

    /// 状态库不可用 → 写以 Err 拒绝（绝不写文件、绝不静默成功），读按默认
    /// 呈现（retire-legacy-state-json 3.2「库不可用呈现 unavailable 而非文件
    /// 派生值」）。
    #[test]
    fn unavailable_store_rejects_writes_and_reads_as_default() {
        let _engine = crate::test_engine::install_none();
        let err = save(&ProviderRuntimeState::default()).unwrap_err();
        assert!(
            err.to_string().contains("不可用"),
            "错误须点名状态库不可用: {err}"
        );
        assert!(update(|s| s.mode = ProviderMode::Router).is_err());
        assert_eq!(load(), ProviderRuntimeState::default());
    }

}
