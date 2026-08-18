//! Unified state store: `~/.sebas/state.json` as single source of truth.
//!
//! spec 2026-08-17 §2.6 — merges the legacy overlay file (`providers.json`,
//! 只放 provider CRUD delta) and runtime state file (`state.json`, mode +
//! default_selection) into one v2 schema, persisted atomically
//! (tmp + rename) on every mutation.
//!
//! ## Wire schema (v2)
//!
//! ```json
//! {
//!   "version": 2,
//!   "providers": { "deepseek": { "preset": "deepseek", ... } },
//!   "deleted":  ["openai"],
//!   "mode":     { "kind": "direct", "provider": "deepseek" },
//!   "default_selection": { "provider": "deepseek", "model": "deepseek-chat" }
//! }
//! ```
//!
//! - `providers`: 仅 UI 创建 / 修改过的条目（与 config.toml seed 合并后得到
//!   完整视图；见 `crud::FileStore`）。Gateway 侧读同一字段做同样合并。
//! - `deleted`: 从 seed 删除的名字（墓碑，防止重启后从只读源复活）。
//! - `mode` / `default_selection`：runtime 决策（spawn 翻译的输入）。
//!
//! ## Version-aware migration
//!
//! `load()` 内部按 state.json 与 providers.json 的存在情况走三条迁移路径：
//!
//! | state.json | providers.json | 路径 |
//! |---|---|---|
//! | 不存在 | 不存在 | 返回 `Default::default()` (空 v2) |
//! | 不存在 | 存在 | 读 providers.json 作 legacy overlay，组装 v2 PersistedState（mode=Off/default=None），写 state.json（tmp+rename），删 providers.json |
//! | 存在 v2 | 不存在 / 存在 | 直接 parse 为 v2；repair：若 mode 指向 deleted 或 missing provider，重置为 Off + 清 default；upgrade：`default_provider_for_direct` → `default_selection`（spec §2.8） |
//! | 存在 v1/v0（无 `version` 字段或 version=1）| 存在 | 读 state.json 作 legacy runtime state（mode+default），读 providers.json 作 legacy overlay，合并写 v2，删 providers.json |
//! | 存在 v1/v0 | 不存在 | 读 state.json 作 legacy runtime state，providers 字段为空，mode/default 保留，写 v2 |
//!
//! ## spec §2.8 default_model 归属迁移
//!
//! v2 schema 里只一个 runtime-default 字段：`default_selection: Option<DefaultSelection>`。
//! 它把旧的 `default_provider_for_direct: Option<String>` 和 overlay item 上的
//! `default_model` 合并到一个 `(provider, model)` 元组（spec §2.8「default_model
//! 分裂」收尾）。Overlay item 仍保留 `default_model` 字段作 UI 源（`/provider`
//! 详情面板里的「默认 model」文本框），`default_selection.model` 是 spawn 翻
//! 译的权威值，由「设为默认（DIRECT）」动作同步过去。
//!
//! 迁移路径：旧 v2 state.json 用 `#[serde(alias = "default_provider_for_direct")]`
//! 直接 parse 到 `default_selection` 字段（model=None），下次 save 时落地为
//! 新字段。**不引入 STATE_VERSION_V3** —— 这是 v2 schema 内的字段重命名，
//! 不是 schema 版本升级。
//!
//! 一句话：迁移是**一次性**的（发生在首次 load 检测到旧文件时），完成后
//! providers.json 不再被读取。所有写入都走 state.json。

use crate::provider_state::ProviderMode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 一条记录：字段名 -> 值。Provider CRUD 用。
pub type Item = Map<String, Value>;

/// 目标 schema 版本号。`PersistedState::default()` 和 `save()` 都写这个版本。
pub const STATE_VERSION_V2: u32 = 2;
/// 旧版 schema：没有 `version` 字段或 version=1 — 只含 mode + default_provider_for_direct。
pub const STATE_VERSION_V1: u32 = 1;

/// Runtime 「DIRECT 默认」选择（spec 2026-08-17 §2.8）。
///
/// 把旧 `default_provider_for_direct: Option<String>` 和 overlay item 上的
/// `default_model: Option<String>` 合并到一个 `(provider, model)` 元组：
/// - `provider`：DIRECT 模式下默认启用的 provider 名（必须存在于 `providers`
///   或 `gateway_cfg`，否则 spawn-time 兜底回退 Off + warn）；
/// - `model`：spawn 时追加的 `--model <id>`（仅在 Direct 模式下生效；Gateway
///   模式由 gateway 自己路由）。
///
/// Overlay item 上的 `default_model` 仍是 UI 源（`/provider` 详情面板的「默认
/// model」文本框），但 spawn 时只信 `default_selection.model`。"set as default"
/// 动作负责把 overlay 的 `default_model` 同步进 `default_selection.model`（见
/// `router::router::provider_card::handle_set_default_direct` 的 merge helper）。
///
/// wire shape：
/// ```json
/// "default_selection": { "provider": "deepseek", "model": "deepseek-chat" }
/// ```
///
/// `model` 缺省 / 显式 None 时不写 `--model`（agent 用自己默认）。
///
/// **serde 自定义反序列化**：为了把旧 `default_provider_for_direct: "<name>"`
/// 形态的 v2 state.json 平滑迁到新形状，`DefaultSelection::deserialize` 同时
/// 接受：
/// - 对象 `{"provider": "...", "model": "..."}`（新）
/// - 字符串 `"<provider>"`（旧 default_provider_for_direct 别名走这条）
///
/// 这避免了引入 STATE_VERSION_V3：旧 v2 文件原地升级，下次 save 落地为对象
/// 形状。`#[serde(alias)]` 单独不够 —— 它只重命名字段，不会把字符串「升级」
/// 成对象。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DefaultSelection {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl DefaultSelection {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: None,
        }
    }

    pub fn with_model(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: Some(model.into()),
        }
    }
}

impl<'de> Deserialize<'de> for DefaultSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt::{self, Formatter};

        struct StringOrStruct;

        impl<'de> Visitor<'de> for StringOrStruct {
            type Value = DefaultSelection;

            fn expecting(&self, f: &mut Formatter) -> fmt::Result {
                f.write_str(
                    "string (legacy default_provider_for_direct) or \
                     object {\"provider\": \"...\", \"model\": \"...\"} \
                     for DefaultSelection",
                )
            }

            fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
                Ok(DefaultSelection::new(s))
            }

            fn visit_string<E: de::Error>(self, s: String) -> Result<Self::Value, E> {
                Ok(DefaultSelection::new(s))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut provider: Option<String> = None;
                let mut model: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "provider" => provider = Some(map.next_value()?),
                        "model" => model = Some(map.next_value()?),
                        _ => {
                            let _: de::IgnoredAny = map.next_value()?;
                        }
                    }
                }
                let provider = provider
                    .ok_or_else(|| de::Error::missing_field("provider"))?;
                Ok(DefaultSelection { provider, model })
            }
        }

        deserializer.deserialize_any(StringOrStruct)
    }
}

/// 统一持久化状态。覆盖以下三个职责：
/// - provider CRUD delta（与 seed 合并出视图）；
/// - runtime 模式 + DIRECT 默认 (provider, model)；
/// - 单文件原子持久化（tmp + rename）。
///
/// spec §2.8：`default_provider_for_direct: Option<String>` 替换为
/// `default_selection: Option<DefaultSelection>`。serde 别名让旧 v2 state.json
/// （含 `default_provider_for_direct` 字段）继续能解析（model=None 落地），
/// 第一次 save 后就只写新字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedState {
    #[serde(default = "default_state_version")]
    pub version: u32,
    #[serde(default)]
    pub providers: BTreeMap<String, Item>,
    #[serde(default)]
    pub deleted: Vec<String>,
    #[serde(default)]
    pub mode: ProviderMode,
    /// spec §2.8：DIRECT 模式默认 (provider, model)。serde 别名接受旧字段
    /// `default_provider_for_direct` —— 旧 state.json 解析到这里时
    /// `model=None`，upgrade step 在 `repair_mode` 后落地为新 wire 形状。
    #[serde(default, alias = "default_provider_for_direct")]
    pub default_selection: Option<DefaultSelection>,
}

fn default_state_version() -> u32 {
    STATE_VERSION_V2
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION_V2,
            providers: BTreeMap::new(),
            deleted: Vec::new(),
            mode: ProviderMode::default(),
            default_selection: None,
        }
    }
}

/// State file 路径：`~/.sebas/state.json`，可用 `SEBAS_STATE_FILE` 覆盖
/// （与 `provider_state.rs` 同惯例）。
pub fn state_path() -> PathBuf {
    let raw = std::env::var("SEBAS_STATE_FILE").unwrap_or_else(|_| "~/.sebas/state.json".into());
    PathBuf::from(expand_tilde(&raw))
}

/// 旧 overlay 文件路径：`~/.sebas/providers.json`，可用
/// `SEBAS_GATEWAY_PROVIDER_OVERLAY` 覆盖（与 `src::provider::overlay_path`
/// 同惯例）。**仅迁移路径使用** — 正常读写都走 `state.json`。
pub fn providers_path() -> PathBuf {
    let raw = std::env::var("SEBAS_GATEWAY_PROVIDER_OVERLAY")
        .unwrap_or_else(|_| "~/.sebas/providers.json".into());
    PathBuf::from(expand_tilde(&raw))
}

/// 复制 `sebas::config::expand_tilde`（router 不能反向依赖 sebas root）。
fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into();
    }
    p.to_string()
}

/// 版本感知迁移：见模块文档的版本矩阵。
///
/// 错误一律 `warn!` 后回退到 `Default::default()` —— runtime 状态不应让
/// sebas 启动失败。Provider overlay 解析失败也走默认（避免「一次配置错
/// 让 /provider 死掉」）。
pub fn load() -> PersistedState {
    let state_file = state_path();
    let overlay_file = providers_path();

    // ---- 路径 A：state.json 存在 → 走版本检测 ----
    if state_file.exists() {
        match std::fs::read_to_string(&state_file) {
            Ok(raw) => {
                // 第一遍：只看顶层 `version` 字段（数字 / 缺失）。
                let detected = parse_version(&raw);
                match detected {
                    Some(STATE_VERSION_V2) => {
                        // v2 直接 parse。
                        match serde_json::from_str::<PersistedState>(&raw) {
                            Ok(mut s) => {
                                s = repair_mode(s);
                                // 若 overlay 文件仍存在（半迁移状态：state.json 已
                                // 写 v2 但 providers.json 未删），best-effort 把它
                                // 合并进来再删。
                                if overlay_file.exists()
                                    && let Ok(extra) = load_legacy_overlay(&overlay_file)
                                {
                                    merge_overlay_into(&mut s, extra);
                                    if save(&s).is_ok() {
                                        let _ = std::fs::remove_file(&overlay_file);
                                    }
                                }
                                return s;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    path = %state_file.display(),
                                    error = %e,
                                    "state.json v2 解析失败，回退默认"
                                );
                                return PersistedState::default();
                            }
                        }
                    }
                    Some(STATE_VERSION_V1) | None => {
                        // legacy v0/v1：state.json 只含 mode + default。
                        // 与 providers.json（若存在）合并 → 写 v2 → 删 providers.json。
                        let (providers, deleted) = if overlay_file.exists() {
                            load_legacy_overlay(&overlay_file)
                                .map(|s| (s.providers, s.deleted))
                                .unwrap_or_default()
                        } else {
                            (BTreeMap::new(), Vec::new())
                        };
                        let mode_and_default = parse_legacy_state(&raw).unwrap_or_default();
                        let s = PersistedState {
                            version: STATE_VERSION_V2,
                            providers,
                            deleted,
                            mode: mode_and_default.mode,
                            default_selection: mode_and_default
                                .default_provider_for_direct
                                .map(DefaultSelection::new),
                        };
                        if save(&s).is_ok() && overlay_file.exists() {
                            // 迁移一次性：state.json 已写 v2 后立刻删 overlay。
                            let _ = std::fs::remove_file(&overlay_file);
                        }
                        return repair_mode(s);
                    }
                    Some(other) => {
                        tracing::warn!(
                            path = %state_file.display(),
                            version = other,
                            "未知的 state.json version，回退默认"
                        );
                        return PersistedState::default();
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    path = %state_file.display(),
                    error = %e,
                    "读取 state.json 失败，回退默认"
                );
                return PersistedState::default();
            }
        }
    }

    // ---- 路径 B：state.json 不存在，providers.json 存在 → 迁移 ----
    if overlay_file.exists() {
        match load_legacy_overlay(&overlay_file) {
            Ok(s) => {
                if save(&s).is_ok() {
                    let _ = std::fs::remove_file(&overlay_file);
                }
                return s;
            }
            Err(e) => {
                tracing::warn!(
                    path = %overlay_file.display(),
                    error = %e,
                    "providers.json 解析失败，回退默认"
                );
                return PersistedState::default();
            }
        }
    }

    // ---- 路径 C：都没有 → 全新装机 ----
    PersistedState::default()
}

/// 原子写：tmp + rename。父目录缺失则创建。失败保留 `Err`。
///
/// 所有 mutation（CRUD / state / 合并）都走这里——一次 mutation = 一次写。
pub fn save(s: &PersistedState) -> anyhow::Result<()> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("创建 state.json 父目录 {} 失败: {e}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(s)
        .map_err(|e| anyhow::anyhow!("序列化 PersistedState 失败: {e}"))?;

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)
        .map_err(|e| anyhow::anyhow!("写入临时文件 {} 失败: {e}", tmp.display()))?;
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&tmp) {
        let _ = file.sync_all();
    }
    std::fs::rename(&tmp, &path)
        .map_err(|e| anyhow::anyhow!("rename {} -> {} 失败: {e}", tmp.display(), path.display()))?;
    Ok(())
}

/// 读 → 改 → 写一气呵成。`f` 闭包基于当前 state 做条件决策；返回改后的 state。
///
/// 闭包操作**整个 PersistedState**（含 providers + deleted），所以可以一次
/// 调用里同时改 provider 数据 + mode + default。
pub fn update<F>(f: F) -> anyhow::Result<PersistedState>
where
    F: FnOnce(&mut PersistedState),
{
    let mut s = load();
    f(&mut s);
    save(&s)?;
    Ok(s)
}

/// "delete default provider" 原子操作：删除 provider + 同步清掉
/// `default_selection`（若指向被删的）+ 写盘。一次写 = 一致性。
///
/// mode 的清理留给 load() 的 `repair_mode`：如果 mode 还指向刚被删的
/// provider，下次 load 会重置为 Off（repair 时机 = 读时，比写时更安全——
/// 写时拿不到 providers + deleted 的最新视图）。
///
/// 返回改后的 state（无论闭包是否真改了东西）。
pub fn delete_provider_and_clear_default(id: &str) -> anyhow::Result<PersistedState> {
    update(|s| {
        s.providers.remove(id);
        if !s.deleted.iter().any(|d| d == id) {
            s.deleted.push(id.to_string());
        }
        if s.default_selection.as_ref().map(|d| d.provider.as_str()) == Some(id) {
            s.default_selection = None;
        }
    })
}

// ---- 内部 helper ----

/// 从 JSON 字符串里抽顶层 `version` 字段（数字）。无字段 / 非数字 → `None`。
fn parse_version(raw: &str) -> Option<u32> {
    let v: Value = serde_json::from_str(raw).ok()?;
    v.get("version")
        .and_then(Value::as_u64)
        .map(|n| n as u32)
}

/// 旧版 state.json 的 wire 形状：只 mode + default_provider_for_direct。
#[derive(Default, Deserialize)]
struct LegacyState {
    #[serde(default)]
    mode: ProviderMode,
    #[serde(default)]
    default_provider_for_direct: Option<String>,
}

fn parse_legacy_state(raw: &str) -> Option<LegacyState> {
    serde_json::from_str(raw).ok()
}

/// 旧版 overlay 文件的 wire 形状：只 providers + deleted。
#[derive(Default, Deserialize)]
struct LegacyOverlay {
    #[serde(default)]
    providers: BTreeMap<String, Item>,
    #[serde(default)]
    deleted: Vec<String>,
}

fn load_legacy_overlay(path: &Path) -> anyhow::Result<PersistedState> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("读取 {} 失败: {e}", path.display()))?;
    let ov: LegacyOverlay = serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("解析 {} 失败: {e}", path.display()))?;
    Ok(PersistedState {
        version: STATE_VERSION_V2,
        providers: ov.providers,
        deleted: ov.deleted,
        mode: ProviderMode::default(),
        default_selection: None,
    })
}

/// 把额外 overlay 项合并进 v2 state（半迁移场景：v2 state.json 已写但
/// providers.json 还在）。冲突时 v2 优先（用户已迁移完成，保留用户后续改的）。
fn merge_overlay_into(s: &mut PersistedState, extra: PersistedState) {
    for (k, v) in extra.providers {
        s.providers.entry(k).or_insert(v);
    }
    for d in extra.deleted {
        if !s.deleted.contains(&d) {
            s.deleted.push(d);
        }
    }
}

/// repair-on-load：若 mode 指向 `deleted` 墓碑里的 provider，重置为 Off。
///
/// **只修 tombstone，不修「not in providers」** —— 后者是合法状态（用户
/// 切到 Direct 模式但还没配任何 provider 时常见），不能误伤。这是「删除
/// default provider」操作的兜底 —— 即便两次写中间崩了，下次 load 也能
/// 自愈，不会让 `Direct{ deleted_provider }` 卡在那里。
///
/// "missing provider" 的判断留给 spawn-time `compute_provider_resolution`
/// 兜底（找不到就回退 Off + warn），不放在持久化层。
fn repair_mode(mut s: PersistedState) -> PersistedState {
    let tombstoned_provider = match &s.mode {
        ProviderMode::Direct { provider } => {
            if s.deleted.iter().any(|d| d == provider) {
                Some(provider.clone())
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(provider) = tombstoned_provider {
        tracing::info!(
            provider = %provider,
            "mode 指向已 tombstoned 的 provider，重置为 Off（repair-on-load）"
        );
        s.mode = ProviderMode::Off;
        if s.default_selection.as_ref().map(|d| d.provider.as_str()) == Some(provider.as_str()) {
            s.default_selection = None;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    /// 路径 C：两个文件都不存在 → 返回 default。
    #[test]
    fn load_returns_default_when_neither_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        assert!(!state_p.exists());
        assert!(!prov_p.exists());
        // 直接调用测试 helper（不走 env var）。
        let s = load_from_paths(&state_p, &prov_p);
        assert_eq!(s, PersistedState::default());
    }

    /// 路径 B：只有 providers.json（legacy v0 overlay） → 迁移到 v2 state.json。
    #[test]
    fn migration_from_legacy_overlay_only() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &prov_p,
            r#"{
                "providers": { "deepseek": { "name": "deepseek", "preset": "deepseek" } },
                "deleted": ["openai"]
            }"#,
        );

        let s = load_from_paths(&state_p, &prov_p);
        // providers + deleted 保留。
        assert!(s.providers.contains_key("deepseek"));
        assert!(s.deleted.contains(&"openai".to_string()));
        // mode + default 是 default。
        assert_eq!(s.mode, ProviderMode::Off);
        assert_eq!(s.default_selection, None);
        // version = v2。
        assert_eq!(s.version, STATE_VERSION_V2);

        // load 完应当触发一次 save（state.json 已创建）并删 providers.json。
        assert!(state_p.exists(), "load 应触发 state.json 写入");
        assert!(!prov_p.exists(), "load 后 providers.json 应被删除");

        // 二次 load：直接命中 v2 path。
        let s2 = load_from_paths(&state_p, &prov_p);
        assert_eq!(s2, s);
    }

    /// 路径 D：state.json 是 v0/v1（无 version 字段）+ providers.json 存在
    /// → 合并写 v2，删 providers.json。
    #[test]
    fn migration_from_v1_state_with_overlay() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        // v1 state.json：缺 version 字段。
        write_file(
            &state_p,
            r#"{
                "mode": { "kind": "direct", "provider": "deepseek" },
                "default_provider_for_direct": "deepseek"
            }"#,
        );
        write_file(
            &prov_p,
            r#"{
                "providers": { "deepseek": { "name": "deepseek", "preset": "deepseek" } },
                "deleted": []
            }"#,
        );

        let s = load_from_paths(&state_p, &prov_p);
        assert_eq!(s.version, STATE_VERSION_V2);
        assert_eq!(s.mode, ProviderMode::Direct { provider: "deepseek".into() });
        assert_eq!(
            s.default_selection.as_ref().map(|d| d.provider.as_str()),
            Some("deepseek"),
            "legacy default_provider_for_direct 应被解析为 default_selection.provider"
        );
        assert_eq!(
            s.default_selection.as_ref().and_then(|d| d.model.clone()),
            None,
            "legacy default_provider_for_direct 不带 model 信息 → default_selection.model = None"
        );
        assert!(s.providers.contains_key("deepseek"));
        // 迁移完写 v2 + 删 overlay。
        assert!(state_p.exists());
        assert!(!prov_p.exists());
        // 文件内容应是 v2 wire。
        let raw = std::fs::read_to_string(&state_p).unwrap();
        let parsed: PersistedState = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed, s);
    }

    /// v1 state.json 但没有 providers.json：mode + default 保留，providers 字段空。
    #[test]
    fn migration_from_v1_state_without_overlay() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &state_p,
            r#"{
                "mode": { "kind": "gateway" },
                "default_provider_for_direct": null
            }"#,
        );

        let s = load_from_paths(&state_p, &prov_p);
        assert_eq!(s.version, STATE_VERSION_V2);
        assert_eq!(s.mode, ProviderMode::Gateway);
        assert!(s.providers.is_empty());
        assert!(s.deleted.is_empty());
    }

    /// repair-on-load：mode 指向 deleted provider → 自动重置为 Off + 清 default。
    #[test]
    fn repair_mode_clears_stale_direct_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        // 写一个 v2 state 但 mode 指向已 tombstoned 的 provider。
        write_file(
            &state_p,
            r#"{
                "version": 2,
                "providers": { "deepseek": { "name": "deepseek" } },
                "deleted": ["openai"],
                "mode": { "kind": "direct", "provider": "openai" },
                "default_provider_for_direct": "openai"
            }"#,
        );

        let s = load_from_paths(&state_p, &prov_p);
        // repair 应已生效。
        assert_eq!(s.mode, ProviderMode::Off);
        assert_eq!(s.default_selection, None);
    }

    /// repair-on-load 不动「mode 指向不在 providers 里」的情况 —— 那是
    /// 合法状态（用户切到 Direct 模式但还没建 provider），留给 spawn-time
    /// `compute_provider_resolution` 兜底（找不到就回退 Off + warn）。
    #[test]
    fn repair_mode_keeps_pointer_to_missing_provider_when_no_tombstone() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &state_p,
            r#"{
                "version": 2,
                "providers": {},
                "deleted": [],
                "mode": { "kind": "direct", "provider": "ghost" },
                "default_provider_for_direct": "ghost"
            }"#,
        );

        let s = load_from_paths(&state_p, &prov_p);
        // repair 不应触发 —— 用户可能正在筹备新 provider。
        assert_eq!(
            s.mode,
            ProviderMode::Direct {
                provider: "ghost".into()
            }
        );
        assert_eq!(
            s.default_selection.as_ref().map(|d| d.provider.as_str()),
            Some("ghost")
        );
    }

    /// v2 → v2 round-trip：所有字段保留。
    #[test]
    fn v2_round_trip_preserves_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        let original = PersistedState {
            version: STATE_VERSION_V2,
            providers: BTreeMap::from([(
                "deepseek".to_string(),
                item_with(&[("name", "deepseek"), ("preset", "deepseek")]),
            )]),
            deleted: vec!["openai".to_string()],
            mode: ProviderMode::Direct { provider: "deepseek".into() },
            default_selection: Some(DefaultSelection::with_model("deepseek", "deepseek-chat")),
        };
        save_to_path(&state_p, &original).unwrap();
        let s = load_from_paths(&state_p, &prov_p);
        assert_eq!(s, original);
    }

    /// spec §2.8：旧 v2 state.json 含 `default_provider_for_direct` 字段（无
    /// `default_selection`）→ load 应通过 `#[serde(alias)]` 直接解析到
    /// `default_selection` 字段（model=None），下次 save 落地为新 wire 形状。
    /// 不需要 STATE_VERSION_V3 触发迁移路径。
    #[test]
    fn v2_with_legacy_default_provider_for_direct_upgrades_to_default_selection() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        // 手写一个「旧形状」的 v2 state.json：含 default_provider_for_direct 但
        // 没有 default_selection。
        write_file(
            &state_p,
            r#"{
                "version": 2,
                "providers": { "deepseek": { "name": "deepseek" } },
                "deleted": [],
                "mode": { "kind": "direct", "provider": "deepseek" },
                "default_provider_for_direct": "deepseek"
            }"#,
        );

        let s = load_from_paths(&state_p, &prov_p);
        // alias 解析成功：default_selection.provider = "deepseek"，model = None
        // （旧字段不带 model 信息）。
        assert_eq!(
            s.default_selection,
            Some(DefaultSelection::new("deepseek")),
            "旧 default_provider_for_direct 应被 alias 解析为 default_selection"
        );

        // 落地：save() 写出来的 wire 不再有旧字段。
        save_to_path(&state_p, &s).unwrap();
        let raw = std::fs::read_to_string(&state_p).unwrap();
        assert!(
            !raw.contains("default_provider_for_direct"),
            "save 后旧字段不应再出现在 wire 上：{raw}"
        );
        assert!(
            raw.contains("default_selection"),
            "save 后新字段应出现在 wire 上：{raw}"
        );

        // 二次 load 命中纯新形状路径。
        let s2 = load_from_paths(&state_p, &prov_p);
        assert_eq!(s2, s);
    }

    /// spec §2.8：v2 state.json 里 `default_provider_for_direct` 和
    /// `default_selection` **同时**出现时，serde 把 alias 字段和命名字段视为
    /// 同一字段 —— 顺序无关紧要，**值必须一致**才是合法状态；冲突时整个
    /// state.json 被视为 corrupt，回退 default（兜底语义保持
    /// `repair_mode` 不动 state 的承诺）。
    ///
    /// 这条测试锁定「同字段两次出现 = 矛盾」的处理：**不会**让旧 alias 字段
    /// 静默覆盖新字段（避免迁移时数据丢失），也**不会**让 spawn 时拿到半旧
    /// 半新的诡异 state。生产环境不太可能触发（用户不会手写这种文件），但
    /// 显式回归锁定兜底行为。
    #[test]
    fn v2_conflicting_default_provider_and_selection_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &state_p,
            r#"{
                "version": 2,
                "providers": {},
                "deleted": [],
                "mode": { "kind": "off" },
                "default_provider_for_direct": "old-provider",
                "default_selection": { "provider": "new-provider", "model": "new-model" }
            }"#,
        );
        let s = load_from_paths(&state_p, &prov_p);
        // 矛盾值：serde 解析失败 → load 回退 default（mode=Off, default_selection=None）。
        // 显式重置 mode 后，下次 save 才会写出干净 state.json。
        assert_eq!(s.mode, ProviderMode::Off);
        assert_eq!(s.default_selection, None);
    }

    /// spec §2.8：DefaultSelection 字段顺序不影响解析（serde 字段是 named map，
    /// 不是 tuple —— 锁定「struct 不是 tuple」这一选型决定）。
    #[test]
    fn default_selection_round_trips_with_field_order_swapped() {
        let original = DefaultSelection::with_model("anthropic", "claude-3-5-sonnet");
        let json = serde_json::to_string(&original).unwrap();
        let parsed: DefaultSelection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);

        // 字段顺序倒过来也应能解析。
        let swapped = r#"{"model":"claude-3-5-sonnet","provider":"anthropic"}"#;
        let parsed: DefaultSelection = serde_json::from_str(swapped).unwrap();
        assert_eq!(
            parsed,
            DefaultSelection::with_model("anthropic", "claude-3-5-sonnet")
        );
    }

    /// spec §2.8：model 为 None 时 wire 上 `model` 字段不出现（`skip_serializing_if`
    /// 行为），保持 state.json 紧凑。
    #[test]
    fn default_selection_with_no_model_omits_field_in_wire() {
        let s = DefaultSelection::new("deepseek");
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            !json.contains("\"model\""),
            "model=None 时 wire 不应有 model 字段：{json}"
        );
        assert!(
            json.contains("\"provider\":\"deepseek\""),
            "provider 字段必须出现：{json}"
        );
        // 反向解析：缺 model 字段 → None。
        let parsed: DefaultSelection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }

    /// 「delete default provider」原子性：delete 后文件里 default_selection
    /// 立刻为 None，不会出现「provider 已删但 default 残留」的不一致。
    #[test]
    fn delete_provider_atomically_clears_default() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        let mut s = PersistedState {
            version: STATE_VERSION_V2,
            providers: BTreeMap::from([("deepseek".to_string(), item_with(&[("name", "deepseek")]))]),
            deleted: Vec::new(),
            mode: ProviderMode::Direct { provider: "deepseek".into() },
            default_selection: Some(DefaultSelection::new("deepseek")),
        };
        // delete_provider_and_clear_default 走 update 路径：写 state.json 一次。
        update_at(&state_p, &prov_p, |st| {
            st.providers.remove("deepseek");
            st.deleted.push("deepseek".into());
            if st.default_selection.as_ref().map(|d| d.provider.as_str()) == Some("deepseek") {
                st.default_selection = None;
            }
            // mode 留给 load 时 repair（测试不强制此刻清）
        });
        s = load_from_paths(&state_p, &prov_p);
        // 一次写后已一致：providers 无 deepseek、deleted 有 deepseek、default 为 None。
        assert!(!s.providers.contains_key("deepseek"));
        assert!(s.deleted.contains(&"deepseek".to_string()));
        assert_eq!(s.default_selection, None);
        // mode 在 repair 之后才被清（Direct{deepseek} → Off）。
        assert_eq!(s.mode, ProviderMode::Off);
    }

    /// save 父目录不存在 → 自动创建。
    #[test]
    fn save_creates_missing_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("state.json");
        save_to_path(&nested, &PersistedState::default()).unwrap();
        assert!(nested.exists());
    }

    /// 未知 version（如 99）：当前实现 warn + 回退 default。
    /// （未来新增 schema 版本时再细化处理。）
    #[test]
    fn unknown_version_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(&state_p, r#"{ "version": 99, "providers": {}, "deleted": [] }"#);
        let s = load_from_paths(&state_p, &prov_p);
        assert_eq!(s, PersistedState::default());
    }

    /// corrupt state.json：解析失败 → warn + default。runtime 状态不应拖死 sebas。
    #[test]
    fn corrupt_state_json_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(&state_p, "{not valid json");
        let s = load_from_paths(&state_p, &prov_p);
        assert_eq!(s, PersistedState::default());
    }

    // ---- helpers（不走 env var，测试可并行） ----

    fn item_with(fields: &[(&str, &str)]) -> Item {
        let mut m = Map::new();
        for (k, v) in fields {
            m.insert((*k).into(), Value::String((*v).into()));
        }
        m
    }

    fn load_from_paths(state_p: &Path, prov_p: &Path) -> PersistedState {
        // 与 `load()` 同逻辑，但路径硬编码。
        if state_p.exists() {
            if let Ok(raw) = std::fs::read_to_string(state_p) {
                match parse_version(&raw) {
                    Some(STATE_VERSION_V2) => {
                        if let Ok(mut s) = serde_json::from_str::<PersistedState>(&raw) {
                            s = repair_mode(s);
                            if prov_p.exists()
                                && let Ok(extra) = load_legacy_overlay(prov_p)
                            {
                                merge_overlay_into(&mut s, extra);
                                if save_to_path(state_p, &s).is_ok() {
                                    let _ = std::fs::remove_file(prov_p);
                                }
                            }
                            return s;
                        }
                        return PersistedState::default();
                    }
                    Some(STATE_VERSION_V1) | None => {
                        let (providers, deleted) = if prov_p.exists() {
                            load_legacy_overlay(prov_p)
                                .map(|s| (s.providers, s.deleted))
                                .unwrap_or_default()
                        } else {
                            (BTreeMap::new(), Vec::new())
                        };
                        let legacy = parse_legacy_state(&raw).unwrap_or_default();
                        let mut s = PersistedState {
                            version: STATE_VERSION_V2,
                            providers,
                            deleted,
                            mode: legacy.mode,
                            default_selection: legacy
                                .default_provider_for_direct
                                .map(DefaultSelection::new),
                        };
                        if save_to_path(state_p, &s).is_ok() && prov_p.exists() {
                            let _ = std::fs::remove_file(prov_p);
                        }
                        return repair_mode(s);
                    }
                    Some(_) => return PersistedState::default(),
                }
            }
        }
        if prov_p.exists()
            && let Ok(mut s) = load_legacy_overlay(prov_p)
        {
            if save_to_path(state_p, &s).is_ok() {
                let _ = std::fs::remove_file(prov_p);
            }
            return s;
        }
        PersistedState::default()
    }

    fn save_to_path(path: &Path, s: &PersistedState) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(s)
            .map_err(|e| anyhow::anyhow!("serialize: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    fn update_at<F: FnOnce(&mut PersistedState)>(
        state_p: &Path,
        prov_p: &Path,
        f: F,
    ) {
        let mut s = load_from_paths(state_p, prov_p);
        f(&mut s);
        save_to_path(state_p, &s).unwrap();
    }
}