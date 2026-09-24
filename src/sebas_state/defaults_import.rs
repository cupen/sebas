//! legacy `defaults.json` 的一次性导入（make-core-own-provider-data D3/1.4）。
//!
//! `defaults.json` 原是 router 独占的文件（`/admin/defaults` 写、core 从不
//! 消费）——正是本 change 移除的第二权威。它是用户选择（默认 provider /
//! model），静默丢弃会丢偏好，因此这是「不读遗留文件」原则的**刻意例外**：
//! core 首次启动时把它的值导入 `settings` 域（`default_selection`，与
//! provider 数据同事务落盘），随后写 `settings` 表的 `defaults_imported`
//! 标记行；标记在场即不再读该文件（哪怕它被旧二进制重新写回）。
//!
//! extract-sebas-db：导入的库内动作（标记行、runtime_state RMW）经
//! `sebas_models::runtime_state` 的域查询完成，本模块只管文件与调度。

use crate::sebas_state::writer::StateHandle;
use sebas_dispatch::state_store::DefaultSelection;
use sebas_models::runtime_state;

/// legacy defaults.json 路径：状态目录下的 `defaults.json`（与旧 router admin
/// 的 `defaults_path` 同一落点）。
///
/// retire-legacy-state-json 3.x：这条派生原本经 `providers.json` 的路径
/// （`SEBAS_ROUTER_PROVIDER_OVERLAY`）算出来，两个变量都已退休；现在直接取
/// 状态目录（`SEBAS_STATE_DIR` 派生），不依赖任何已退休的 env。
pub fn legacy_defaults_path() -> std::path::PathBuf {
    sebas_domain::state_paths::state_dir().join("defaults.json")
}

/// legacy defaults 文件的 wire 形状（与旧 router `read_defaults` 同一解析：
/// `{"provider": "...", "model": "..." | null}`；provider 空 = 未设置）。
#[derive(serde::Deserialize)]
struct LegacyDefaults {
    provider: String,
    #[serde(default)]
    model: Option<String>,
}

/// 启动期一次性导入（定位经状态目录派生——两个 legacy 文件 env 已退休）。
pub async fn import_legacy_defaults_once(handle: &StateHandle) -> Result<bool, String> {
    import_legacy_defaults_from(handle, &legacy_defaults_path()).await
}

/// 显式路径形态：逻辑与上同，测试直接传路径（不碰全局 env，避免与并行
/// 测试的 env 重定向互踩）。返回是否实际导入了值（仅用于测试断言）。
///
/// - 标记已存在 → 直接返回（**不读文件**——「之后不再读」由标记保证）；
/// - 文件缺失 → 落标记、返回 false（导入阶段已完成）；
/// - 文件损坏 / provider 为空 → warn 一行（留档）后照常落标记：文件时代
///   已结束，不值得每次启动重放告警；
/// - 成功 → `default_selection` 与标记行同事务写入（与 provider 数据同一
///   次提交），info 一行日志。
pub async fn import_legacy_defaults_from(
    handle: &StateHandle,
    path: &std::path::Path,
) -> Result<bool, String> {
    if handle
        .exec(runtime_state::defaults_import_done)
        .await?
    {
        return Ok(false);
    }
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<LegacyDefaults>(&raw).ok())
        .filter(|d| !d.provider.trim().is_empty());
    let Some(legacy) = parsed else {
        // 缺失/损坏/空 provider：不导入值，但导入阶段完成（落标记）。
        if path.exists() {
            tracing::warn!(
                path = %path.display(),
                "legacy defaults.json 存在但无法解析（provider 缺失或 JSON 损坏），跳过导入"
            );
        }
        handle.exec(runtime_state::mark_defaults_imported).await?;
        return Ok(false);
    };
    let provider = legacy.provider.trim().to_string();
    let model = legacy
        .model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    let selection = match model {
        Some(m) => DefaultSelection::with_model(provider, m),
        None => DefaultSelection::new(provider),
    };
    let imported = handle
        .exec(move |conn| runtime_state::import_defaults_once(conn, selection))
        .await?;
    if imported {
        tracing::info!(
            path = %path.display(),
            "legacy defaults.json 已一次性导入 settings 域（default_selection）；此后不再读取该文件"
        );
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sebas_state::writer::StateWriter;

    /// 测试直接传显式路径，不碰全局 env（与并行测试的 env 重定向互踩会
    /// 造成偶发失败）。
    fn defaults_file(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("defaults.json")
    }

    #[tokio::test]
    async fn imports_once_then_never_reads_again() {
        let dir = tempfile::tempdir().unwrap();
        let defaults = defaults_file(dir.path());
        std::fs::write(
            &defaults,
            r#"{"provider": "deepseek", "model": "deepseek-chat"}"#,
        )
        .unwrap();

        let writer = StateWriter::start(dir.path().join("import.db")).unwrap();
        // 第一次：导入成功。
        assert!(
            import_legacy_defaults_from(writer.handle(), &defaults)
                .await
                .unwrap()
        );
        let state = writer
            .handle()
            .exec(crate::sebas_state::repo::load_persisted_state)
            .await
            .unwrap();
        assert_eq!(
            state.default_selection,
            Some(DefaultSelection::with_model("deepseek", "deepseek-chat"))
        );

        // 改文件内容（模拟旧二进制回头改写）→ 第二次启动不得重读、不得覆盖。
        std::fs::write(&defaults, r#"{"provider": "other", "model": null}"#).unwrap();
        assert!(
            !import_legacy_defaults_from(writer.handle(), &defaults)
                .await
                .unwrap()
        );
        let state = writer
            .handle()
            .exec(crate::sebas_state::repo::load_persisted_state)
            .await
            .unwrap();
        assert_eq!(
            state.default_selection,
            Some(DefaultSelection::with_model("deepseek", "deepseek-chat")),
            "标记在场后文件变化不得影响库"
        );
    }

    #[tokio::test]
    async fn absent_file_completes_the_import_phase_without_values() {
        let dir = tempfile::tempdir().unwrap();
        let defaults = defaults_file(dir.path());
        let writer = StateWriter::start(dir.path().join("import-absent.db")).unwrap();
        assert!(
            !import_legacy_defaults_from(writer.handle(), &defaults)
                .await
                .unwrap()
        );
        let state = writer
            .handle()
            .exec(crate::sebas_state::repo::load_persisted_state)
            .await
            .unwrap();
        assert_eq!(state.default_selection, None);
        // 导入阶段已完成：后续调用不再读文件（即便此刻文件出现）。
        std::fs::write(&defaults, r#"{"provider": "late", "model": null}"#).unwrap();
        assert!(
            !import_legacy_defaults_from(writer.handle(), &defaults)
                .await
                .unwrap()
        );
        let state = writer
            .handle()
            .exec(crate::sebas_state::repo::load_persisted_state)
            .await
            .unwrap();
        assert_eq!(state.default_selection, None);
    }
}
