//! Beads CLI client（agent-task-center 1.3/1.4）.
//!
//! sebas 通过 spawn `bd --json` 使用 beads：**绝不直接读写 Dolt 库**——
//! 那是 `bd` 的独占写入面（「一个文件一个写入者」；绕过 CLI 会破坏 claim
//! 原子性、lease 与 events journal 一致性）。本模块是唯一的 `bd` 调用点，
//! 把 CLI 的 stdout 解析成类型化 Rust 值，把失败分类成 [`BeadsError`]。
//!
//! 二进制发现：`SEBAS_BD_BIN` 环境变量（沙箱/测试用假桩）优先，缺省 `bd`
//! 走 `PATH`。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// 覆盖 `bd` 可执行文件的环境变量（测试/沙箱指向假桩）。
pub const BD_BIN_ENV: &str = "SEBAS_BD_BIN";

/// 常规命令的超时。`bd` 单次操作实测 ~50ms；10s 是「卡死即失败」的
/// 兜底，不是性能预算。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// 单次 `bd events tail` 的超时（design D5：200ms，防偶发慢查询卡轮询）。
pub const EVENTS_TAIL_TIMEOUT: Duration = Duration::from_millis(200);

/// `bd` 调用失败的类型化分类（task 1.4）。
#[derive(Debug, thiserror::Error)]
pub enum BeadsError {
    /// 二进制不存在 / 不可执行（HTTP 503：部署缺失，不是请求错误）。
    #[error("bd 未安装或不可执行: {0}")]
    NotInstalled(String),
    /// 非零退出（HTTP 500：命令失败；具体原因在 stderr 原文里）。
    #[error("bd 命令失败 (exit {code}): {stderr}")]
    CommandFailed {
        code: i32,
        stdout: String,
        stderr: String,
    },
    /// stdout 不是期望的 JSON 形状（HTTP 500：CLI 契约漂移）。
    #[error("bd 输出无法解析为 JSON: {0}")]
    Parse(String),
    /// 命令超时（HTTP 503：`bd` 暂时不可用，可重试）。
    #[error("bd 命令超时")]
    Timeout,
    /// `--claim` 撞上他人的活认领（HTTP 409，点名当前持有者）。
    #[error("任务已被 {holder} 认领")]
    AlreadyClaimed { holder: String },
    /// 目标 issue 不存在（HTTP 404）。
    #[error("任务不存在: {id}")]
    NotFound { id: String },
    /// events journal 已被保留期裁剪（读到这里之前的序号不再可用）。
    ///
    /// `floor` = 最老的仍留存序号（pump 从 `floor - 1` 继续，缺口靠手动
    /// Refresh 收敛）；`head` = 历史最高分配序号——**手动 Refresh 的恢复
    /// 基线**（P2-2：截断时 `--since 0` 必失败，只有 `head` 能把游标
    /// 钉到「从现在开始」）。
    #[error("beads events journal 已截断（floor={floor}, head={head}）")]
    JournalTruncated { floor: i64, head: i64 },
}

impl BeadsError {
    /// 失败是「暂时性」的（轮询循环据此退避而非放弃）。
    pub fn is_transient(&self) -> bool {
        matches!(self, BeadsError::Timeout | BeadsError::CommandFailed { .. })
    }
}

/// `bd create/show/list/update/close` 的 issue 行形状（`--json` 输出的
/// 公共子集；`--brief` 之外的字段一律容错缺省——CLI 版本漂移不丢帧）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BeadsIssue {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// claim 的 lease 面（burn-down 展示与死工人恢复的观测点）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    /// 关闭原因（`--reason cancelled` 时前端据「Done / Cancelled」分区）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// issue 的自定义元数据（`reviewer` 存这里——design D4）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// `bd show --include-comments` 才携带评论正文。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<BeadsComment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependent_count: Option<i64>,
}

impl BeadsIssue {
    /// 该 issue 是否处于 `in_review`（beads 原生 status 之外用 `in-review`
    /// label 表达——design D4）。
    pub fn is_in_review(&self) -> bool {
        self.labels.iter().any(|l| l == "in-review")
    }

    /// reviewer 元数据字段（`--metadata '{"reviewer":"…"}'`）。
    pub fn reviewer(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .and_then(|m| m.get("reviewer"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
    }
}

/// 一条评论（`bd comment` / `bd comments --json`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BeadsComment {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub issue_id: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub created_at: String,
}

/// events journal 的一条记录（`bd events tail --json` 的 NDJSON 行）。
///
/// 载荷含**完整 issue 快照**（`issue`）——前端收到直接替换本地状态，无需
/// 回查 `bd show`（design D5）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BeadsEventRecord {
    pub seq: i64,
    #[serde(default)]
    pub ts: String,
    #[serde(default)]
    pub op: String,
    #[serde(default)]
    pub issue_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<BeadsIssue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<BeadsComment>,
}

/// 创建任务的请求（HTTP 层校验后原样交给 `bd create`）。
#[derive(Debug, Clone, Default)]
pub struct CreateTaskRequest {
    pub title: String,
    pub description: Option<String>,
    pub assignee: Option<String>,
    /// 创建时绑定的 reviewer（存 issue 的 `--metadata '{"reviewer":…}'`）。
    pub reviewer: Option<String>,
    /// 额外标签（如 `in-review`）。
    pub labels: Vec<String>,
}

/// `bd list` 的过滤条件。
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<String>,
    pub assignee: Option<String>,
}

/// `bd --json` 的异步封装。
#[derive(Debug, Clone)]
pub struct BeadsClient {
    bin: PathBuf,
}

impl Default for BeadsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl BeadsClient {
    /// 从 `SEBAS_BD_BIN`（测试/沙箱）或 `PATH` 上的 `bd` 构造。
    pub fn new() -> Self {
        let bin = std::env::var_os(BD_BIN_ENV)
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("bd"));
        Self { bin }
    }

    /// 显式指定二进制（单元测试用假桩）。
    pub fn with_bin(bin: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into() }
    }

    /// 底层执行口：`bd -C <dir> --json [--actor <actor>] <args…>`。
    async fn run(
        &self,
        dir: &Path,
        actor: Option<&str>,
        args: &[&str],
        timeout: Duration,
    ) -> Result<String, BeadsError> {
        let mut cmd = Command::new(&self.bin);
        // bd v1.3.4 的全局 flag 面：`-C` / `--json` / `--no-color` /
        // `--actor` 均有；**没有** `--no-pager`（未知 flag 会整条命令失败）。
        cmd.arg("-C").arg(dir).arg("--json").arg("--no-color");
        if let Some(actor) = actor.filter(|a| !a.trim().is_empty()) {
            cmd.arg("--actor").arg(actor);
        }
        cmd.args(args);
        self.spawn(cmd, timeout).await
    }

    /// 在项目目录**内**执行（无 `-C`）：`bd init` 的 `-C` 会因「目录还不是
    /// beads 项目」被拒——初始化只能先切 cwd。
    async fn run_in_dir(
        &self,
        dir: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<String, BeadsError> {
        let mut cmd = Command::new(&self.bin);
        cmd.current_dir(dir).arg("--json").arg("--no-color");
        cmd.args(args);
        self.spawn(cmd, timeout).await
    }

    /// spawn + 超时 + ETXTBSY 重试 + 非零退出分类。
    async fn spawn(&self, mut cmd: Command, timeout: Duration) -> Result<String, BeadsError> {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // 超时/丢弃 future 时回收子进程，绝不留孤儿 `bd`。
            .kill_on_drop(true);

        let output = {
            // ETXTBSY：刚写出的可执行脚本在 fork 竞态下可能短暂处于「正在
            // 写入」状态（测试假桩；生产 `bd` 不会）。与 agent_kinds 的
            // `output_with_etxtbsy_retry` 同一处理：短暂重试而不是误报缺失。
            let mut attempts = 0;
            loop {
                match tokio::time::timeout(timeout, cmd.output()).await {
                    Err(_) => return Err(BeadsError::Timeout),
                    Ok(Ok(output)) => break output,
                    Ok(Err(e)) if is_etxtbsy(&e) && attempts < 5 => {
                        attempts += 1;
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Ok(Err(e)) => {
                        return Err(BeadsError::NotInstalled(format!(
                            "{}: {e}",
                            self.bin.display()
                        )));
                    }
                }
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            return Err(classify_failure(
                output.status.code().unwrap_or(-1),
                &stdout,
                &stderr,
            ));
        }
        Ok(stdout)
    }

    /// 解析单个 JSON 值（`create` / `comment` / `assign` 等对象输出）。
    async fn run_json<T>(
        &self,
        dir: &Path,
        actor: Option<&str>,
        args: &[&str],
        timeout: Duration,
    ) -> Result<T, BeadsError>
    where
        T: serde::de::DeserializeOwned,
    {
        let out = self.run(dir, actor, args, timeout).await?;
        serde_json::from_str(out.trim())
            .map_err(|e| BeadsError::Parse(format!("{e}; raw={}", out.trim())))
    }

    /// 解析 JSON 数组（`create --json` 的数组形态、`list` / `show` /
    /// `update` / `close` 都是数组——`bd` 的 `--json` 契约按命令不同）。
    async fn run_json_array<T>(
        &self,
        dir: &Path,
        actor: Option<&str>,
        args: &[&str],
        timeout: Duration,
    ) -> Result<Vec<T>, BeadsError>
    where
        T: serde::de::DeserializeOwned,
    {
        let out = self.run(dir, actor, args, timeout).await?;
        let trimmed = out.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        // 兼容两种形状：顶层数组，或单个对象（部分命令单条输出）。
        if trimmed.starts_with('[') {
            serde_json::from_str(trimmed)
                .map_err(|e| BeadsError::Parse(format!("{e}; raw={trimmed}")))
        } else {
            let one: T = serde_json::from_str(trimmed)
                .map_err(|e| BeadsError::Parse(format!("{e}; raw={trimmed}")))?;
            Ok(vec![one])
        }
    }

    // ─── 初始化（lazy bootstrap，task 2.1） ────────────────────────────────

    /// 惰性初始化：`.beads/` 不存在时 `bd init --prefix <basename 前 4 字符>`，
    /// 已存在则只确保 `events-journal = true`（不重复 init——用户可能已手动
    /// 建过库）。幂等。
    pub async fn ensure_initialized(&self, project_dir: &Path) -> Result<(), BeadsError> {
        let beads_dir = project_dir.join(".beads");
        if !beads_dir.exists() {
            let prefix = derive_prefix(project_dir);
            // `bd init` 必须在项目目录内、且不带 `-C`（`-C` 要求目标已是
            // beads 项目）。
            self.run_in_dir(
                project_dir,
                &["init", "--prefix", &prefix, "--non-interactive"],
                DEFAULT_TIMEOUT,
            )
            .await?;
        }
        // 无论新建还是已存在，都强制打开 events journal（实时泵的数据源）。
        self.run(
            project_dir,
            None,
            &["config", "set", "events-journal", "true"],
            DEFAULT_TIMEOUT,
        )
        .await?;
        Ok(())
    }

    // ─── 只读 ──────────────────────────────────────────────────────────────

    /// `bd list --all --json [--status s] [--assignee a]`。
    pub async fn list(
        &self,
        dir: &Path,
        filter: &TaskFilter,
    ) -> Result<Vec<BeadsIssue>, BeadsError> {
        let mut args: Vec<&str> = vec!["list", "--all"];
        if let Some(status) = filter.status.as_deref().filter(|s| !s.is_empty()) {
            args.push("--status");
            args.push(status);
        }
        if let Some(assignee) = filter.assignee.as_deref().filter(|s| !s.is_empty()) {
            args.push("--assignee");
            args.push(assignee);
        }
        self.run_json_array(dir, None, &args, DEFAULT_TIMEOUT).await
    }

    /// `bd show <id> --include-comments --json`（返回完整 issue 含评论）。
    pub async fn show(&self, dir: &Path, id: &str) -> Result<BeadsIssue, BeadsError> {
        let mut issues: Vec<BeadsIssue> = self
            .run_json_array(
                dir,
                None,
                &["show", id, "--include-comments"],
                DEFAULT_TIMEOUT,
            )
            .await?;
        issues
            .pop()
            .ok_or_else(|| BeadsError::NotFound { id: id.to_string() })
    }

    /// `bd comments <id> --json`。
    pub async fn comments(&self, dir: &Path, id: &str) -> Result<Vec<BeadsComment>, BeadsError> {
        self.run_json_array(dir, None, &["comments", id], DEFAULT_TIMEOUT)
            .await
    }

    // ─── 变更 ──────────────────────────────────────────────────────────────

    /// `bd create --json`（task 3.2）。reviewer 落 `--metadata`。
    pub async fn create(
        &self,
        dir: &Path,
        actor: &str,
        req: &CreateTaskRequest,
    ) -> Result<BeadsIssue, BeadsError> {
        let mut args: Vec<String> = vec![
            "create".into(),
            req.title.clone(),
            "--type".into(),
            "task".into(),
        ];
        if let Some(desc) = req.description.as_deref().filter(|d| !d.trim().is_empty()) {
            args.push("--description".into());
            args.push(desc.to_string());
        }
        if let Some(assignee) = req.assignee.as_deref().filter(|a| !a.is_empty()) {
            args.push("--assignee".into());
            args.push(assignee.to_string());
        }
        if let Some(reviewer) = req.reviewer.as_deref().filter(|r| !r.is_empty()) {
            args.push("--metadata".into());
            args.push(serde_json::json!({ "reviewer": reviewer }).to_string());
        }
        if !req.labels.is_empty() {
            args.push("--labels".into());
            args.push(req.labels.join(","));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        // `bd create --json` 输出单个对象（实测 v1.3.4）。
        let issue: BeadsIssue = self
            .run_json(dir, Some(actor), &refs, DEFAULT_TIMEOUT)
            .await?;
        Ok(issue)
    }

    /// `bd update <id> --claim --json`（task 3.4）。原子认领：被自己认领
    /// 幂等成功；被他人活认领 → [`BeadsError::AlreadyClaimed`]。
    pub async fn claim(&self, dir: &Path, actor: &str, id: &str) -> Result<BeadsIssue, BeadsError> {
        self.run_json_array::<BeadsIssue>(
            dir,
            Some(actor),
            &["update", id, "--claim"],
            DEFAULT_TIMEOUT,
        )
        .await?
        .pop()
        .ok_or_else(|| BeadsError::NotFound { id: id.to_string() })
    }

    /// `bd unclaim <id> --json`（task 3.5）：清空 assignee、status 回 `open`。
    pub async fn unclaim(
        &self,
        dir: &Path,
        actor: &str,
        id: &str,
    ) -> Result<BeadsIssue, BeadsError> {
        self.run_json_array::<BeadsIssue>(dir, Some(actor), &["unclaim", id], DEFAULT_TIMEOUT)
            .await?
            .pop()
            .ok_or_else(|| BeadsError::NotFound { id: id.to_string() })
    }

    /// `bd assign <id> <name> --json`（task 3.6）。
    pub async fn assign(
        &self,
        dir: &Path,
        actor: &str,
        id: &str,
        assignee: &str,
    ) -> Result<BeadsIssue, BeadsError> {
        let issue: BeadsIssue = self
            .run_json(dir, Some(actor), &["assign", id, assignee], DEFAULT_TIMEOUT)
            .await?;
        Ok(issue)
    }

    /// `bd update <id> --add-label <label> --json`（task 3.7：进入评审）。
    pub async fn add_label(
        &self,
        dir: &Path,
        actor: &str,
        id: &str,
        label: &str,
    ) -> Result<BeadsIssue, BeadsError> {
        self.run_json_array::<BeadsIssue>(
            dir,
            Some(actor),
            &["update", id, "--add-label", label],
            DEFAULT_TIMEOUT,
        )
        .await?
        .pop()
        .ok_or_else(|| BeadsError::NotFound { id: id.to_string() })
    }

    /// `bd close <id> [--reason r] --json`（task 3.8）。
    ///
    /// `force` 用于 webui 的 reviewer 闸门已放行、但 bd 自身的 actor 不等于
    /// assignee 的场景（`bd close` 默认拒绝关闭他人活认领）。闸门在 WebUI
    /// 层（design D10），这里只负责把已裁决的动作执行到底。
    pub async fn close(
        &self,
        dir: &Path,
        actor: &str,
        id: &str,
        reason: Option<&str>,
        force: bool,
    ) -> Result<BeadsIssue, BeadsError> {
        let mut args: Vec<&str> = vec!["close", id];
        if let Some(reason) = reason.filter(|r| !r.is_empty()) {
            args.push("--reason");
            args.push(reason);
        }
        if force {
            args.push("--force");
        }
        self.run_json_array::<BeadsIssue>(dir, Some(actor), &args, DEFAULT_TIMEOUT)
            .await?
            .pop()
            .ok_or_else(|| BeadsError::NotFound { id: id.to_string() })
    }

    /// `bd comment <id> <text> --json`（task 3.9）。
    pub async fn comment(
        &self,
        dir: &Path,
        actor: &str,
        id: &str,
        text: &str,
    ) -> Result<BeadsComment, BeadsError> {
        self.run_json(dir, Some(actor), &["comment", id, text], DEFAULT_TIMEOUT)
            .await
    }

    // ─── 实时事件（task 4.1） ──────────────────────────────────────────────

    /// `bd events tail --since <seq> --json`：严格增量、按 seq 升序返回。
    ///
    /// **必须用 `--since` flag**——spike 实测位置参数被静默忽略（design D5
    /// 踩坑）。单次调用有独立超时（200ms），无新事件时 `bd` 立即返回空。
    pub async fn events_tail(
        &self,
        dir: &Path,
        since: i64,
        timeout: Duration,
    ) -> Result<Vec<BeadsEventRecord>, BeadsError> {
        let since_str = since.to_string();
        let out = self
            .run(
                dir,
                None,
                &["events", "tail", "--since", &since_str],
                timeout,
            )
            .await?;
        let mut records = Vec::new();
        for line in out.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<BeadsEventRecord>(line) {
                Ok(record) => records.push(record),
                // 非 JSON 的提示行（CLI 版本差异）跳过而不是让整次轮询失败。
                Err(e) => tracing::debug!(error = %e, line, "bd events tail: 跳过无法解析的行"),
            }
        }
        records.sort_by_key(|r| r.seq);
        Ok(records)
    }
}

/// 把 issue 前缀派生为 `bd init --prefix` 合法值：basename 的字母数字，
/// 小写、取前 4 个；全空时回退 `tsk`。
fn derive_prefix(project_dir: &Path) -> String {
    let base = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let prefix: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(4)
        .collect::<String>()
        .to_ascii_lowercase();
    if prefix.is_empty() {
        "tsk".to_string()
    } else {
        prefix
    }
}

/// ETXTBSY（Linux 26）：刚写出的可执行脚本在 fork 竞态下短暂不可执行。
/// 只在 spawn 失败时用（重试而不是误报「未安装」）。
fn is_etxtbsy(e: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        e.raw_os_error() == Some(26)
    }
    #[cfg(not(unix))]
    {
        let _ = e;
        false
    }
}

/// 非零退出 → 类型化错误。claim 撞车与事件 journal 截断是两类需要**语义化**
/// 的失败（调用方要 409 点名持有者 / 用 floor+head 重新基线），其余归
/// CommandFailed。
fn classify_failure(code: i32, stdout: &str, stderr: &str) -> BeadsError {
    let combined = format!("{stdout}\n{stderr}");
    if combined.contains("events_journal_truncated") {
        let floor = extract_json_i64(&combined, "floor");
        // `head` 缺席时退化为 floor（旧 bd 形态）：仍能重新基线，只是可能
        // 重放 floor..head 的记录（幂等替换，无一致性代价）。
        let head = extract_json_i64(&combined, "head").or(floor);
        if let (Some(floor), Some(head)) = (floor, head) {
            return BeadsError::JournalTruncated { floor, head };
        }
        return BeadsError::CommandFailed {
            code,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        };
    }
    if let Some(holder) = extract_holder(&combined) {
        return BeadsError::AlreadyClaimed { holder };
    }
    if combined.contains("not found")
        || combined.contains("no issues found matching the provided IDs")
        || combined.contains("no issue found")
    {
        return BeadsError::NotFound {
            id: extract_issue_id(&combined).unwrap_or_default(),
        };
    }
    BeadsError::CommandFailed {
        code,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    }
}

/// 从 bd 的报错原文里抽当前持有者。bd 1.3.4 的四种措辞：
///
/// - `already assigned to "claude"`（assign 撞车）
/// - `issue already claimed by alice`（claim 撞车）
/// - `cannot reassign x: held by "alice" (in_progress)`（assign 他人活认领）
/// - `issue claimed by a different actor: x is held by bob; …`（unclaim）
fn extract_holder(text: &str) -> Option<String> {
    for marker in [
        "already assigned to \"",
        "already claimed by ",
        "held by \"",
        "is held by ",
    ] {
        if let Some(idx) = text.find(marker) {
            let rest = &text[idx + marker.len()..];
            let end = if marker.ends_with('"') {
                rest.find('"').unwrap_or(rest.len())
            } else {
                rest.find(|c: char| {
                    c.is_whitespace() || c == ',' || c == '—' || c == ';' || c == '('
                })
                .unwrap_or(rest.len())
            };
            let holder = rest[..end].trim();
            if !holder.is_empty() {
                return Some(holder.to_string());
            }
        }
    }
    None
}

/// 从任意 JSON 片段里取一个整数字段（journal 截断错误的 `floor` / `head`）。
fn extract_json_i64(text: &str, field: &str) -> Option<i64> {
    let needle = format!("\"{field}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = text[start..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// 从 bd 的「找不到 issue」原文里回填 id（P3-1：`NotFound{id}` 曾恒为空，
/// 错误文案退化成「任务不存在: 」）。只认 `alpha-num` 段 + 连字符的 issue
/// id 形状，避免把普通单词当 id。
fn extract_issue_id(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|token| {
            token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        })
        .find(|token| {
            let mut parts = token.split('-');
            let Some(first) = parts.next() else {
                return false;
            };
            !first.is_empty()
                && first.chars().all(|c| c.is_ascii_alphanumeric())
                && parts.clone().next().is_some()
                && parts
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// 写一个 POSIX shell 假 `bd`：按第一个非全局参数分派，输出预置 JSON。
    /// 同时把 argv 追加到 `<dir>/bd-invocations.log`（用于断言 init 次数）。
    #[cfg(unix)]
    fn fake_bd(script_body: &str) -> (TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bd");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            "#!/bin/sh\n\
             echo \"$@\" >> \"$(dirname \"$0\")/invocations.log\"\n\
             {script_body}\n"
        )
        .unwrap();
        drop(f);
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        (dir, path)
    }

    #[cfg(unix)]
    fn project(dir: &TempDir) -> PathBuf {
        let p = dir.path().join("proj");
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn create_parses_single_object() {
        let body = r#"cat <<'EOF'
{"id":"tsk-1","title":"T","status":"open","priority":2,"issue_type":"task","metadata":{"reviewer":"claude"}}
EOF"#;
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        let issue = client
            .create(
                &project(&dir),
                "alice",
                &CreateTaskRequest {
                    title: "T".into(),
                    description: Some("D".into()),
                    assignee: None,
                    reviewer: Some("claude".into()),
                    labels: vec![],
                },
            )
            .await
            .unwrap();
        assert_eq!(issue.id, "tsk-1");
        assert_eq!(issue.reviewer(), Some("claude"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn list_parses_array_and_passes_filters() {
        let body = r#"cat <<'EOF'
[{"id":"a","title":"A","status":"open"},{"id":"b","title":"B","status":"in_progress","assignee":"claude","labels":["in-review"],"comment_count":2}]
EOF"#;
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        let proj = project(&dir);
        let issues = client
            .list(
                &proj,
                &TaskFilter {
                    status: Some("open".into()),
                    assignee: Some("claude".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(issues.len(), 2);
        assert!(issues[1].is_in_review());
        let log = std::fs::read_to_string(dir.path().join("invocations.log")).unwrap();
        assert!(
            log.contains("list --all --status open --assignee claude"),
            "{log}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn show_returns_issue_with_comments() {
        let body = r#"cat <<'EOF'
[{"id":"a","title":"A","status":"open","comments":[{"id":"c1","issue_id":"a","author":"alice","text":"hi","created_at":"t"}]}]
EOF"#;
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        let issue = client.show(&project(&dir), "a").await.unwrap();
        assert_eq!(issue.comments.len(), 1);
        assert_eq!(issue.comments[0].author, "alice");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claim_conflict_names_the_holder() {
        let body = r#"echo 'Error updating tsk-1: issue already claimed: already assigned to "claude" — coordinate' >&2
exit 1"#;
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        let err = client
            .claim(&project(&dir), "bob", "tsk-1")
            .await
            .unwrap_err();
        match err {
            BeadsError::AlreadyClaimed { holder } => assert_eq!(holder, "claude"),
            other => panic!("expected AlreadyClaimed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn events_tail_parses_ndjson_and_sorts() {
        let body = r#"cat <<'EOF'
{"seq":2,"op":"update","issue_id":"a","issue":{"id":"a","title":"A","status":"in_progress"}}
{"seq":1,"op":"create","issue_id":"a","issue":{"id":"a","title":"A","status":"open"}}
EOF"#;
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        let records = client
            .events_tail(&project(&dir), 0, DEFAULT_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].seq, 1, "按 seq 升序");
        assert_eq!(records[1].issue.as_ref().unwrap().status, "in_progress");
        let log = std::fs::read_to_string(dir.path().join("invocations.log")).unwrap();
        assert!(log.contains("events tail --since 0"), "{log}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_truncation_carries_floor_and_head() {
        let body = r#"echo '{"error":"events_journal_truncated","since":0,"floor":7,"head":20,"code":"events_journal_truncated"}' >&2
exit 1"#;
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        match client.events_tail(&project(&dir), 0, DEFAULT_TIMEOUT).await {
            Err(BeadsError::JournalTruncated { floor, head }) => {
                assert_eq!((floor, head), (7, 20));
            }
            other => panic!("expected JournalTruncated, got {other:?}"),
        }
    }

    /// P3-1：bd 1.3.4 的 assign / unclaim 撞车措辞都必须归到
    /// `AlreadyClaimed`（否则调用方拿到 500 而不是点名持有者的 409）。
    #[tokio::test]
    async fn assign_and_unclaim_conflicts_name_the_holder() {
        // assign：`cannot reassign x: held by "alice" (in_progress)`
        let body = r#"echo 'cannot reassign tsk-1: held by "alice" (in_progress)' >&2
exit 1"#;
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        match client.assign(&project(&dir), "bob", "tsk-1", "carol").await {
            Err(BeadsError::AlreadyClaimed { holder }) => assert_eq!(holder, "alice"),
            other => panic!("expected AlreadyClaimed, got {other:?}"),
        }

        // unclaim：`issue claimed by a different actor: x is held by bob; …`
        let body = r#"echo 'issue claimed by a different actor: tsk-1 is held by bob; reclaim or use --force to override' >&2
exit 1"#;
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        match client.unclaim(&project(&dir), "alice", "tsk-1").await {
            Err(BeadsError::AlreadyClaimed { holder }) => assert_eq!(holder, "bob"),
            other => panic!("expected AlreadyClaimed, got {other:?}"),
        }
    }

    /// P3-1：NotFound 的 id 从 bd 原文回填（错误文案不再退化成空串）。
    #[tokio::test]
    async fn not_found_error_carries_the_issue_id() {
        let body = r#"echo 'Issue tsk-nope not found' >&2
exit 1"#;
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        match client.show(&project(&dir), "tsk-nope").await {
            Err(BeadsError::NotFound { id }) => assert_eq!(id, "tsk-nope"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_binary_is_not_installed() {
        let client = BeadsClient::with_bin("/nonexistent/bd-binary-xyz");
        let err = client.list(Path::new("/tmp"), &TaskFilter::default()).await;
        assert!(matches!(err, Err(BeadsError::NotInstalled(_))), "{err:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_zero_exit_is_command_failed() {
        let body = "echo 'boom' >&2\nexit 3";
        let (dir, bin) = fake_bd(body);
        let client = BeadsClient::with_bin(bin);
        match client.list(&project(&dir), &TaskFilter::default()).await {
            Err(BeadsError::CommandFailed { code, stderr, .. }) => {
                assert_eq!(code, 3);
                assert!(stderr.contains("boom"));
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_json_stdout_is_parse_error() {
        let (dir, bin) = fake_bd("echo 'not json'");
        let client = BeadsClient::with_bin(bin);
        assert!(matches!(
            client.list(&project(&dir), &TaskFilter::default()).await,
            Err(BeadsError::Parse(_))
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_is_reported_and_child_reaped() {
        let (dir, bin) = fake_bd("sleep 5");
        let client = BeadsClient::with_bin(bin);
        let err = client
            .events_tail(&project(&dir), 0, Duration::from_millis(50))
            .await;
        assert!(matches!(err, Err(BeadsError::Timeout)), "{err:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_initialized_inits_once_then_only_forces_journal() {
        let (dir, bin) = fake_bd("exit 0");
        let client = BeadsClient::with_bin(bin);
        let proj = project(&dir);
        client.ensure_initialized(&proj).await.unwrap();
        // 假桩不建 .beads，模拟「已初始化」需要手动创建。
        std::fs::create_dir_all(proj.join(".beads")).unwrap();
        client.ensure_initialized(&proj).await.unwrap();

        let log = std::fs::read_to_string(dir.path().join("invocations.log")).unwrap();
        let init_count = log.matches("init --prefix").count();
        let config_count = log.matches("config set events-journal true").count();
        assert_eq!(init_count, 1, "第二次访问不得重复 init: {log}");
        assert_eq!(config_count, 2, "每次都确保 events-journal: {log}");
        assert!(
            log.contains("--prefix proj"),
            "前缀取 basename 前 4 字符: {log}"
        );
    }
}
