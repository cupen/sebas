//! 工具抽象（task 3.1）与注册表。六件套实现在子模块中。

pub mod bash;
pub mod fs_ops;
pub mod search;

use crate::llm::ToolSchema;
use crate::message::ToolOutput;
use crate::policy::{Approver, PolicyEngine, SandboxMode};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// 工具执行上下文：随会话创建、随 turn 换发新的取消令牌。
pub struct ToolCtx {
    /// 会话工作目录（webui 传入 project_dir）。
    pub workdir: PathBuf,
    /// 本 turn 的取消令牌（C7）；bash 等长任务据此自我终止。
    pub cancel: CancellationToken,
    /// 会话级已读文件集合（write/edit 的 read-before-write 门控）。
    pub read_files: Arc<Mutex<HashSet<PathBuf>>>,
    /// 策略引擎（None = 不做策略门控，1a 行为）。
    pub policy: Option<Arc<PolicyEngine>>,
    /// 审批回答者（None + Ask 判定 = fail closed 拒绝）。
    pub approver: Option<Arc<dyn Approver>>,
}

impl ToolCtx {
    pub fn new(workdir: PathBuf, cancel: CancellationToken) -> Self {
        Self {
            workdir,
            cancel,
            read_files: Arc::new(Mutex::new(HashSet::new())),
            policy: None,
            approver: None,
        }
    }

    pub fn mark_read(&self, p: &Path) {
        self.read_files.lock().expect("read_files poisoned").insert(p.to_path_buf());
    }

    pub fn was_read(&self, p: &Path) -> bool {
        self.read_files.lock().expect("read_files poisoned").contains(p)
    }
}

/// 统一工具接口（D2）：name / description / parameters 三元组直接映射为
/// Anthropic `tools` 数组条目；错误是数据（C4），绝不 panic。
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;

    /// 何时用 / 何时不用——写给模型看的契约。
    fn description(&self) -> String;

    /// JSON Schema（映射 Anthropic tool.input_schema）。
    fn parameters(&self) -> serde_json::Value;

    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx) -> ToolOutput;
}

/// 六件套注册表。
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// 按 spec 顺序注册六个工具；`bash_timeout` 为 bash 缺省时限，
    /// bash 沙箱档位默认 Auto（Landlock 可用即用，否则防火墙）。
    pub fn new(bash_timeout: Duration) -> Self {
        Self::with_sandbox(bash_timeout, SandboxMode::Auto)
    }

    /// 同 [`Self::new`]，显式指定 bash 沙箱档位（design N2 配置面）。
    pub fn with_sandbox(bash_timeout: Duration, sandbox: SandboxMode) -> Self {
        Self {
            tools: vec![
                Arc::new(bash::BashTool::new(bash_timeout, sandbox)),
                Arc::new(fs_ops::ReadTool),
                Arc::new(fs_ops::WriteTool),
                Arc::new(fs_ops::EditTool),
                Arc::new(search::GlobTool),
                Arc::new(search::GrepTool),
            ],
        }
    }

    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .iter()
            .map(|t| ToolSchema {
                name: t.name().to_string(),
                description: t.description(),
                parameters: t.parameters(),
            })
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.iter().map(|t| t.name()).collect()
    }
}

/// 把路径解析到会话工作目录下（绝对路径按原样使用）。
pub(crate) fn resolve_under(workdir: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workdir.join(path)
    }
}

/// walk 时跳过的目录名（.git / target / node_modules）。
pub(crate) fn is_ignored_dir_name(name: &str) -> bool {
    name == ".git" || name == "target" || name == "node_modules"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// task 3.1 验证：注册表恰好包含六件套，且每个 schema 都是合法的
    /// JSON Schema object（properties/required 形状，映射 input_schema）。
    #[test]
    fn registry_lists_all_six_tools_with_valid_json_schemas() {
        let registry = ToolRegistry::new(std::time::Duration::from_secs(60));
        assert_eq!(
            registry.names(),
            vec!["bash", "read", "write", "edit", "glob", "grep"]
        );
        let schemas = registry.schemas();
        assert_eq!(schemas.len(), 6);
        for s in &schemas {
            assert!(!s.name.is_empty());
            assert!(s.description.len() > 20, "description is model-facing contract");
            assert_eq!(s.parameters["type"], "object");
            assert!(s.parameters["properties"].is_object());
            if let Some(required) = s.parameters["required"].as_array() {
                assert!(
                    required.iter().all(|r| r.is_string()),
                    "required must be string array"
                );
            }
        }
        // get 往返
        for name in ["bash", "read", "write", "edit", "glob", "grep"] {
            assert!(registry.get(name).is_some(), "{name} must be registered");
        }
        assert!(registry.get("nonexistent").is_none());
    }
}
