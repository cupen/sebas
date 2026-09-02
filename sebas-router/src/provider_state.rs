//! `/provider` 运行态持久化：mode + default_selection。
//!
//! 自 state.json v2 统一起（openspec/specs/provider-management/spec.md，背景见
//! docs/design-history.md ADR-4），这部分数据合并进 `state.json`（见
//! `state_store::PersistedState`）。本模块保留 `ProviderRuntimeState` 类型
//! 与 `load()` / `update()` 自由函数 API（向后兼容），但底层都委托给
//! `state_store` —— 不再单独读写 `state.json`。
//!
//! 写入频率：仅在 `/provider` 命令切换时更新（典型 <1 次/天），所以用
//! 同步 std fs + tempfile + atomic rename 就够了，不用 async / mutex。

use crate::state_store::{self, DefaultSelection, PersistedState};
use serde::{Deserialize, Serialize};

/// Provider 路由模式（runtime 决策）：
/// - `Off`：直连 sebas 自带的 default 模型，跳过 gateway。
/// - `Direct { provider }`：把请求路由到名为 `provider` 的 provider，
///   但不经过 gateway（直连上游）。
/// - `Gateway`：所有请求走 gateway（gateway 自己负责选 provider）。
///
/// 注意：与 gateway 内部 `GatewayConfig.mode`（`off`/`upstream`）语义
/// 不完全相同 —— 这里是 sebas 这一侧对 spawn 路径的开关。
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderMode {
    /// Default = Off（无网关配置时诚实降级）。
    #[default]
    Off,
    Direct { provider: String },
    Gateway,
}



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

/// 状态文件路径：`~/.sebas/state.json`，可用 `SEBAS_STATE_FILE` 覆盖。
/// （委托给 `state_store` 单一权威路径。）
pub fn state_path() -> std::path::PathBuf {
    state_store::state_path()
}

/// `SEBAS_STATE_FILE` env 覆盖：让测试和隔离部署走自己的 state 文件，
/// 不污染 `~/.sebas/state.json`。
#[cfg(test)]
mod env_override_tests {
    use super::*;
    use crate::state_store::{self, PersistedState};
    use crate::test_util::lock_state_file;

    // 串行化所有 env 访问：和 spawn_env.rs 同源问题。

    #[test]
    fn sebas_state_file_env_overrides_state_path() {
        let _g = lock_state_file();
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("custom_state.json");
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("SEBAS_STATE_FILE", &custom);
        }

        // state_path() 现在返回 env 指定的路径。
        let resolved = state_path();
        assert_eq!(
            resolved, custom,
            "SEBAS_STATE_FILE 应覆盖默认 ~/.sebas/state.json"
        );

        // load() 在新文件不存在时返回默认。
        let loaded = load();
        assert_eq!(loaded, ProviderRuntimeState::default());

        // save() / load() 在 env 路径上往返成功。
        let mut s = ProviderRuntimeState::default();
        s.mode = ProviderMode::Direct {
            provider: "env-override".into(),
        };
        s.default_selection = Some(DefaultSelection::new("env-override"));
        save(&s).expect("save to env-override path");
        assert!(custom.exists(), "save 应创建 env 指定的文件");
        let reloaded = load();
        assert_eq!(
            reloaded.mode,
            ProviderMode::Direct {
                provider: "env-override".into()
            }
        );
        assert_eq!(
            reloaded
                .default_selection
                .as_ref()
                .map(|d| d.provider.as_str()),
            Some("env-override")
        );

        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("SEBAS_STATE_FILE");
        }
        // 让其他测试看到 default path（state_store 已被污染写一个 v2 文件，
        // 但默认 path 不存在 → load 仍返回 default）。
        let _ = state_store::load();
    }
}

/// 读盘并解析。失败语义（文件缺失、解析错误、IO 错）一律 warn 后返回
/// `Default::default()` —— runtime 状态不应让 sebas 启动失败。
pub fn load() -> ProviderRuntimeState {
    ProviderRuntimeState::from(&state_store::load())
}

/// 原子写入：先写 `<path>.tmp` 再 rename。父目录缺失则创建。
///
/// `rename` 在同一文件系统上是原子的，避免半截写入；失败时 tmp 残留
/// 不致命，下次 save 会覆盖。
///
/// **重要**：这个 save 会把当前 PersistedState 整体覆盖（包括 providers +
/// deleted 字段）。调用方应该先 load → 改 → save，或者用 `update()` 闭包。
pub fn save(s: &ProviderRuntimeState) -> anyhow::Result<()> {
    let mut current = state_store::load();
    s.apply_to(&mut current);
    state_store::save(&current)
}

/// 读 → 改 → 写一气呵成。`update` 闭包基于当前 runtime state 做条件决策。
///
/// 返回写盘后的最新 runtime state。
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

/// 供测试与未来 mock 注入用：把给定 path 设进 `SEBAS_STATE_FILE`。
/// 不暴露给生产调用方 —— 测试用 env var 切就行。
#[doc(hidden)]
pub fn set_state_path_for_test(path: &std::path::Path) {
    // SAFETY: tests using this helper should serialize via TEST_LOCK.
    unsafe {
        std::env::set_var("SEBAS_STATE_FILE", path.to_str().unwrap());
    }
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
            ProviderMode::Gateway,
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
            serde_json::to_value(ProviderMode::Gateway).unwrap(),
            serde_json::json!({"kind": "gateway"})
        );
    }

    /// save → load 往返保留全部字段。
    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let _g = set_state_file_for_test(&path);
        let original = ProviderRuntimeState {
            mode: ProviderMode::Direct {
                provider: "anthropic".into(),
            },
            default_selection: Some(DefaultSelection::with_model("deepseek", "deepseek-chat")),
        };
        save(&original).unwrap();
        let loaded = load();
        assert_eq!(loaded, original);
        unset_state_file_for_test();
    }

    /// `update()` 读 → 改 → 写都做完了，且返回值就是改后的状态。
    #[test]
    fn update_mutates_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let _g = set_state_file_for_test(&path);
        let updated = update(|s| {
            s.mode = ProviderMode::Gateway;
            s.default_selection = Some(DefaultSelection::new("openai"));
        })
        .unwrap();
        assert_eq!(updated.mode, ProviderMode::Gateway);
        assert_eq!(
            updated
                .default_selection
                .as_ref()
                .map(|d| d.provider.as_str()),
            Some("openai")
        );
        let reloaded = load();
        assert_eq!(reloaded, updated);
        unset_state_file_for_test();
    }

    /// save 父目录不存在时自动创建 —— 首次部署友好。
    #[test]
    fn save_creates_missing_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("state.json");
        let _g = set_state_file_for_test(&nested);
        save(&ProviderRuntimeState::default()).unwrap();
        assert!(nested.exists());
        unset_state_file_for_test();
    }

    // ---- helpers（test-only env var 切换，串行用） ----

    /// 锁住 STATE_FILE_LOCK, 设置 env, 返回 guard（guard 存活期间锁保持）。
    fn set_state_file_for_test(path: &std::path::Path) -> std::sync::MutexGuard<'static, ()> {
        let g = crate::test_util::lock_state_file();
        // SAFETY: lock held.
        unsafe {
            std::env::set_var("SEBAS_STATE_FILE", path.to_str().unwrap());
        }
        g
    }

    fn unset_state_file_for_test() {
        // SAFETY: lock held.
        unsafe {
            std::env::remove_var("SEBAS_STATE_FILE");
        }
    }
}
