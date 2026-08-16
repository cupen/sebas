//! `/provider` 运行态持久化：mode + default_provider_for_direct。
//!
//! 区别于 `provider.rs`：那里管的是 provider 数据本身（name、base_url、
//! api_key 等），写到 `~/.sebas/providers.json`。这里是 runtime 控制面
//! ——「当前走哪条路径」，写到独立的 `~/.sebas/state.json`，互不干扰。
//!
//! 写入频率：仅在 `/provider` 命令切换时更新（典型 <1 次/天），所以用
//! 同步 std fs + tempfile + atomic rename 就够了，不用 async / mutex。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 复制 sebas::config::expand_tilde（router 不能反向依赖 sebas root）。
/// 简单 ~/foo → $HOME/foo 替换，不支持 ~user 形式。
fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into();
    }
    p.to_string()
}

/// Provider 路由模式（runtime 决策）：
/// - `Off`：直连 sebas 自带的 default 模型，跳过 gateway。
/// - `Direct { provider }`：把请求路由到名为 `provider` 的 provider，
///   但不经过 gateway（直连上游）。
/// - `Gateway`：所有请求走 gateway（gateway 自己负责选 provider）。
///
/// 注意：与 gateway 内部 `GatewayConfig.mode`（`off`/`upstream`）语义
/// 不完全相同 —— 这里是 sebas 这一侧对 spawn 路径的开关。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderMode {
    Off,
    Direct { provider: String },
    Gateway,
}

impl Default for ProviderMode {
    fn default() -> Self {
        Self::Off
    }
}

/// 运行时持久化状态。落到 `~/.sebas/state.json`。
///
/// 设计要点：
/// - 字段都 `#[serde(default)]`：旧文件缺字段时仍能加载（向前兼容）。
/// - 整结构 `Default`：第一次跑没有 state.json 时 `load()` 直接返回这个。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRuntimeState {
    #[serde(default)]
    pub mode: ProviderMode,
    #[serde(default)]
    pub default_provider_for_direct: Option<String>,
}

/// 状态文件路径：`~/.sebas/state.json`，可用 `SEBAS_STATE_FILE` 覆盖（与
/// `SEBAS_GATEWAY_PROVIDER_OVERLAY` 同惯例 —— 测试 / 隔离部署时指向别的目录）。
pub fn state_path() -> PathBuf {
    let raw = std::env::var("SEBAS_STATE_FILE").unwrap_or_else(|_| "~/.sebas/state.json".into());
    PathBuf::from(expand_tilde(&raw))
}

/// `SEBAS_STATE_FILE` env 覆盖：让测试和隔离部署走自己的 state 文件，
/// 不污染 `~/.sebas/state.json`。
#[cfg(test)]
mod env_override_tests {
    use super::*;
    use std::sync::Mutex;

    // 串行化所有 env 访问：和 spawn_env.rs 同源问题。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn sebas_state_file_env_overrides_state_path() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("custom_state.json");
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("SEBAS_STATE_FILE", &custom);
        }

        // state_path() 现在返回 env 指定的路径。
        let resolved = state_path();
        assert_eq!(resolved, custom, "SEBAS_STATE_FILE 应覆盖默认 ~/.sebas/state.json");

        // load() 在新文件不存在时返回默认。
        let loaded = load();
        assert_eq!(loaded, ProviderRuntimeState::default());

        // save() / load() 在 env 路径上往返成功。
        let mut s = ProviderRuntimeState::default();
        s.mode = ProviderMode::Direct { provider: "env-override".into() };
        s.default_provider_for_direct = Some("env-override".into());
        save(&s).expect("save to env-override path");
        assert!(custom.exists(), "save 应创建 env 指定的文件");
        let reloaded = load();
        assert_eq!(reloaded.mode, ProviderMode::Direct { provider: "env-override".into() });
        assert_eq!(reloaded.default_provider_for_direct.as_deref(), Some("env-override"));

        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("SEBAS_STATE_FILE");
        }
    }
}

/// 读盘并解析。失败语义（文件缺失、解析错误、IO 错）一律 warn 后返回
/// `Default::default()` —— runtime 状态不应让 sebas 启动失败。
pub fn load() -> ProviderRuntimeState {
    let path = state_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<ProviderRuntimeState>(&s) {
            Ok(st) => st,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "state.json 解析失败，返回默认 runtime state"
                );
                ProviderRuntimeState::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProviderRuntimeState::default(),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "state.json 读取失败，返回默认 runtime state"
            );
            ProviderRuntimeState::default()
        }
    }
}

/// 原子写入：先写 `<path>.tmp` 再 rename。父目录缺失则创建。
///
/// `rename` 在同一文件系统上是原子的，避免半截写入；失败时 tmp 残留
/// 不致命，下次 save 会覆盖。
pub fn save(s: &ProviderRuntimeState) -> anyhow::Result<()> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!("创建 state.json 父目录 {} 失败: {e}", parent.display())
        })?;
    }
    let body = serde_json::to_string_pretty(s)
        .map_err(|e| anyhow::anyhow!("序列化 ProviderRuntimeState 失败: {e}"))?;

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)
        .map_err(|e| anyhow::anyhow!("写入临时文件 {} 失败: {e}", tmp.display()))?;
    // best-effort fsync：崩溃后状态最多丢这一次更新
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&tmp) {
        let _ = file.sync_all();
    }
    std::fs::rename(&tmp, &path).map_err(|e| {
        anyhow::anyhow!("rename {} -> {} 失败: {e}", tmp.display(), path.display())
    })?;
    Ok(())
}

/// 读 → 改 → 写一气呵成。`update` 闭包可以基于当前 state 做条件决策。
///
/// 返回写盘后的最新 state（即使闭包没有改动也返回，方便调用方继续用）。
pub fn update<F>(f: F) -> anyhow::Result<ProviderRuntimeState>
where
    F: FnOnce(&mut ProviderRuntimeState),
{
    let mut s = load();
    f(&mut s);
    save(&s)?;
    Ok(s)
}

/// 供测试与未来 mock 注入用：强制覆盖 `state_path()` 的结果。
/// 不暴露给生产调用方 —— 通过 env var 切就行。
#[doc(hidden)]
pub fn state_path_at(path: &Path) -> PathBuf {
    PathBuf::from(expand_tilde(&path.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_off_with_no_default_provider() {
        let s = ProviderRuntimeState::default();
        assert_eq!(s.mode, ProviderMode::Off);
        assert_eq!(s.default_provider_for_direct, None);
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

    /// `load()` 在文件不存在时返回默认 —— 首次跑 sebas 不应崩。
    /// 用 `tempfile` 把 SEBAS_STATE_FILE 指向临时目录验证。
    #[test]
    fn load_returns_default_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        // SEBAS_STATE_FILE 是 free function 用的 env；这里直接走 file-level API。
        let s = load_from_path(&path);
        assert_eq!(s, ProviderRuntimeState::default());
    }

    /// save → load 往返保留全部字段。
    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let original = ProviderRuntimeState {
            mode: ProviderMode::Direct {
                provider: "anthropic".into(),
            },
            default_provider_for_direct: Some("deepseek".into()),
        };
        save_to_path(&path, &original).unwrap();
        let loaded = load_from_path(&path);
        assert_eq!(loaded, original);
    }

    /// 坏 JSON 不应崩，warn + 默认 —— runtime 状态不能把 sebas 拖死。
    #[test]
    fn load_with_corrupt_json_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{not valid json").unwrap();
        let s = load_from_path(&path);
        assert_eq!(s, ProviderRuntimeState::default());
    }

    /// `update()` 读 → 改 → 写都做完了，且返回值就是改后的状态。
    #[test]
    fn update_mutates_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        // 空文件起步。
        let updated = update_at(&path, |s| {
            s.mode = ProviderMode::Gateway;
            s.default_provider_for_direct = Some("openai".into());
        })
        .unwrap();
        assert_eq!(updated.mode, ProviderMode::Gateway);
        assert_eq!(updated.default_provider_for_direct.as_deref(), Some("openai"));
        // 独立 load 验证持久化。
        let reloaded = load_from_path(&path);
        assert_eq!(reloaded, updated);
    }

    /// 旧文件缺字段时仍能加载（`#[serde(default)]`）。
    #[test]
    fn load_tolerates_partial_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"mode":{"kind":"off"}}"#).unwrap();
        let s = load_from_path(&path);
        assert_eq!(s.mode, ProviderMode::Off);
        assert_eq!(s.default_provider_for_direct, None);
    }

    /// save 父目录不存在时自动创建 —— 首次部署友好。
    #[test]
    fn save_creates_missing_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("state.json");
        save_to_path(&nested, &ProviderRuntimeState::default()).unwrap();
        assert!(nested.exists());
    }

    // ---- helpers (file-level, 不依赖 env，测试可并行 / 不污染全局) ----

    fn load_from_path(path: &Path) -> ProviderRuntimeState {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => ProviderRuntimeState::default(),
        }
    }

    fn save_to_path(path: &Path, s: &ProviderRuntimeState) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(s)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    fn update_at<F>(path: &Path, f: F) -> anyhow::Result<ProviderRuntimeState>
    where
        F: FnOnce(&mut ProviderRuntimeState),
    {
        let mut s = load_from_path(path);
        f(&mut s);
        save_to_path(path, &s)?;
        Ok(s)
    }
}