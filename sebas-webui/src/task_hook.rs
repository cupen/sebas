//! 会话终态 → 任务进入评审的联动钩子（agent-task-center 5.1~5.3）.
//!
//! 引擎（`sebas-dispatch`）在会话终态处完成全部裁决（Closed/Exited、最后一轮
//! 正常完成、该会话是任务的最新推进会话），本模块只负责执行联动：给 key 对应
//! 的 beads 任务加 `in-review` 标签。
//!
//! **fire-and-forget**：钩子实现同步返回、内部 `tokio::spawn` 异步执行，失败只
//! 记日志——`bd` 的慢/失败绝不阻塞会话生命周期主循环（spec 5.3）。

use crate::beads_client::BeadsClient;
use crate::task_routes::IN_REVIEW_LABEL;
use sebas_dispatch::{SessionTerminalHook, SessionTerminalTask};
use std::path::PathBuf;
use std::sync::Arc;

/// 生产钩子：每次终结启动一条异步 `bd update --add-label in-review`。
pub fn make_hook(client: BeadsClient) -> SessionTerminalHook {
    Arc::new(move |task: SessionTerminalTask| {
        let client = client.clone();
        tokio::spawn(async move {
            let Some(dir) = task.project_dir.as_deref().map(PathBuf::from) else {
                tracing::warn!(
                    task_id = %task.task_id,
                    key = %task.key,
                    "会话终态联动跳过：会话未绑定项目目录（无从执行 bd）"
                );
                return;
            };
            let actor = task.actor.as_deref().filter(|a| !a.trim().is_empty()).unwrap_or("");
            match client
                .add_label(&dir, actor, &task.task_id, IN_REVIEW_LABEL)
                .await
            {
                Ok(issue) => tracing::info!(
                    task_id = %task.task_id,
                    status = %issue.status,
                    reason = task.reason,
                    "会话终态联动：任务已进入评审"
                ),
                Err(error) => tracing::warn!(
                    task_id = %task.task_id,
                    key = %task.key,
                    %error,
                    "会话终态联动失败（任务状态保持不变）"
                ),
            }
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    fn fake_bd(dir: &std::path::Path, calls: &std::path::Path, script_body: &str) -> PathBuf {
        let bin = dir.join("bd");
        let mut f = std::fs::File::create(&bin).unwrap();
        write!(
            f,
            "#!/bin/sh\necho \"$@\" >> {}\n{}\n",
            calls.display(),
            script_body
        )
        .unwrap();
        f.sync_all().unwrap();
        drop(f);
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    /// 5.2/5.3 验收：钩子同步返回、异步给任务加 `in-review`；调用形状是
    /// `update <task_id> --add-label in-review` 且带 actor。
    #[tokio::test]
    async fn hook_adds_in_review_label_without_blocking() {
        let tmp = tempfile::tempdir().unwrap();
        let calls = tmp.path().join("calls.log");
        let bin = fake_bd(
            tmp.path(),
            &calls,
            r#"echo '[{"id":"bd-42","title":"t","status":"in_progress","labels":["in-review"]}]'"#,
        );
        let hook = make_hook(BeadsClient::with_bin(bin));
        let task = SessionTerminalTask {
            channel: "web".into(),
            key: "web-key".into(),
            task_id: "bd-42".into(),
            project_dir: Some(tmp.path().display().to_string()),
            actor: Some("claude".into()),
            reason: "closed",
        };
        hook(task);

        // fire-and-forget：轮询等异步任务落一次调用（不 sleep 固定时长）。
        let mut seen = String::new();
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(&calls) {
                seen = text;
                if !seen.trim().is_empty() {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            seen.contains("update bd-42 --add-label in-review"),
            "hook must run bd update with the in-review label: {seen:?}"
        );
        assert!(seen.contains("--actor claude"), "actor rides the bd call: {seen:?}");
    }
}
