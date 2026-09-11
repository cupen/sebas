//! 控制面侧的操作者级材料仓（add-remote-execution-node 7.3 / 7.4）。
//!
//! 它是**来向请求**的处理器：节点在会话创建时向控制面要材料（`FetchMaterials`），
//! 控制面在这里应答完整内容。变更通知（`MaterialsChanged`）只带版本号——**内容只走
//! 应答**，因为通知是"顺手广播"，不该把内容灌给所有节点。
//!
//! 版本语义：`set_bundle` 换版本就是"出了一版新材料"。节点侧钉住旧版本的会话**不受
//! 影响**（可复现），新会话拿到新版本。

use crate::node_link::client::InboundHandler;
use sebas_node_link::{MaterialFile, SessionOp, SessionResult, SessionRejectCode};
use std::sync::{Arc, RwLock};

/// 一版材料。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Bundle {
    version: String,
    files: Vec<MaterialFile>,
}

/// 控制面材料仓。
#[derive(Debug, Default)]
pub struct MaterialStore {
    bundle: RwLock<Option<Bundle>>,
}

impl MaterialStore {
    /// 空仓（尚未配置材料）。空仓时节点拉取会得到**如实拒绝**，而不是空包——
    /// "没有材料"与"材料是空的"是两件事，前者不该被当成后者。
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 设置当前版本。所有路径先按**共享规则**校验；不合法就整批拒绝（不部分写入）。
    pub fn set_bundle(
        &self,
        version: impl Into<String>,
        mut files: Vec<MaterialFile>,
    ) -> Result<(), String> {
        let version = version.into();
        if version.trim().is_empty() {
            return Err("材料版本号不能为空".into());
        }
        for file in &files {
            sebas_node_link::validate_material_path(&file.path)
                .map_err(|cause| format!("{}: {cause}", file.path))?;
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        *self.bundle.write().unwrap_or_else(|e| e.into_inner()) = Some(Bundle { version, files });
        Ok(())
    }

    /// 清空（回到"未配置"）。
    pub fn clear(&self) {
        *self.bundle.write().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// 当前版本（未配置 → `None`）。
    pub fn version(&self) -> Option<String> {
        self.bundle
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|b| b.version.clone())
    }

    /// 当前版本的文件数。
    pub fn file_count(&self) -> usize {
        self.bundle
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|b| b.files.len())
            .unwrap_or(0)
    }

    fn current(&self) -> Option<Bundle> {
        self.bundle.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[async_trait::async_trait]
impl InboundHandler for MaterialStore {
    async fn handle(&self, op: SessionOp) -> SessionResult {
        match op {
            SessionOp::FetchMaterials { version } => match self.current() {
                None => SessionResult::Rejected {
                    code: SessionRejectCode::NodeError,
                    cause: "控制面尚未配置操作者级材料".into(),
                },
                Some(bundle) => match version {
                    // 节点点名要的版本已经不在（被替换）→ 如实拒绝，不悄悄给新版：
                    // 会话的可复现性依赖"拿到的是我要的那一版"。
                    Some(want) if want != bundle.version => SessionResult::Rejected {
                        code: SessionRejectCode::NodeError,
                        cause: format!(
                            "请求的材料版本 {want} 不在控制面上（当前 {}）",
                            bundle.version
                        ),
                    },
                    _ => SessionResult::Materials {
                        version: bundle.version,
                        files: bundle.files,
                    },
                },
            },
            other => SessionResult::Rejected {
                code: SessionRejectCode::NodeError,
                cause: format!("控制面不处理该来向请求：{other:?}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, content: &str) -> MaterialFile {
        MaterialFile {
            path: path.into(),
            content: content.into(),
        }
    }

    #[tokio::test]
    async fn an_unconfigured_store_refuses_instead_of_serving_an_empty_bundle() {
        let store = MaterialStore::new();
        assert_eq!(store.version(), None);
        match store
            .handle(SessionOp::FetchMaterials { version: None })
            .await
        {
            SessionResult::Rejected { cause, .. } => {
                assert!(cause.contains("尚未配置"), "{cause}")
            }
            other => panic!("空仓应如实拒绝，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_configured_store_serves_the_current_version_sorted() {
        let store = MaterialStore::new();
        store
            .set_bundle(
                "v1",
                vec![file("z.md", "z"), file("a/skill.md", "a")],
            )
            .unwrap();
        assert_eq!(store.version().as_deref(), Some("v1"));
        assert_eq!(store.file_count(), 2);

        match store
            .handle(SessionOp::FetchMaterials { version: None })
            .await
        {
            SessionResult::Materials { version, files } => {
                assert_eq!(version, "v1");
                assert_eq!(files[0].path, "a/skill.md", "按路径排序，便于比对");
                assert_eq!(files[1].path, "z.md");
            }
            other => panic!("{other:?}"),
        }
        // 点名要当前版本也照给。
        assert!(matches!(
            store
                .handle(SessionOp::FetchMaterials {
                    version: Some("v1".into())
                })
                .await,
            SessionResult::Materials { .. }
        ));
    }

    #[tokio::test]
    async fn a_missing_version_is_refused_not_silently_upgraded() {
        let store = MaterialStore::new();
        store.set_bundle("v2", vec![file("a.md", "two")]).unwrap();
        match store
            .handle(SessionOp::FetchMaterials {
                version: Some("v1".into()),
            })
            .await
        {
            SessionResult::Rejected { cause, .. } => {
                assert!(cause.contains("v1"), "{cause}");
                assert!(cause.contains("v2"), "{cause}");
            }
            other => panic!("点名旧版本应如实拒绝，实际 {other:?}"),
        }
    }

    #[test]
    fn set_bundle_validates_paths_as_a_whole() {
        let store = MaterialStore::new();
        assert!(
            store
                .set_bundle("v1", vec![file("ok.md", "x"), file("../evil.md", "x")])
                .is_err(),
            "非法路径整批拒绝"
        );
        assert_eq!(store.version(), None, "拒绝后不留半版");
        assert!(store.set_bundle("  ", vec![]).is_err(), "版本号不能为空");
    }

    #[tokio::test]
    async fn other_inbound_requests_are_refused() {
        let store = MaterialStore::new();
        store.set_bundle("v1", vec![]).unwrap();
        match store.handle(SessionOp::Ping).await {
            SessionResult::Rejected { cause, .. } => {
                assert!(cause.contains("不处理"), "{cause}")
            }
            other => panic!("{other:?}"),
        }
    }
}
