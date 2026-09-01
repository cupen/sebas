//! bash 工具（task 3.2，design N6）：进程组 spawn、超时/取消 killpg、
//! 尾部 30k 截断、非零退出码 = `ok:true` 携带 `exit_code`（spec「Model
//! recovers from a failed command」——失败是给模型看的数据，不是循环中断）。

use super::{Tool, ToolCtx};
use crate::message::{ToolErrorKind, ToolOutput};
use std::time::Duration;

/// bash 输出上限：尾部 30k（spec「size-capped with truncation indicated」）。
pub const BASH_OUTPUT_CAP: usize = 30_000;

/// 管道捕获上限（design N6）：超限后继续排空管道但不累积，防大输出撑爆内存。
const MAX_CAPTURE_BYTES: usize = 1_000_000;

pub struct BashTool {
    default_timeout: Duration,
}

impl BashTool {
    pub fn new(default_timeout: Duration) -> Self {
        Self {
            default_timeout,
        }
    }
}

#[async_trait::async_trait]
impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "bash"
    }

    fn description(&self) -> String {
        "Run a shell command in the session working directory and return its combined \
         stdout+stderr. Prefer targeted commands (ls, grep-like via the grep tool, git status) \
         over long pipelines. A nonzero exit code is reported in the result so you can react \
         to it; the command is killed after the tool timeout."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to run."},
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional per-call timeout in seconds."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(command) = input.get("command").and_then(|c| c.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `command` string");
        };
        let timeout = input
            .get("timeout_secs")
            .and_then(|t| t.as_u64())
            .map(Duration::from_secs)
            .unwrap_or(self.default_timeout);

        run_shell(command, &ctx.workdir, timeout, &ctx.cancel).await
    }
}

/// spawn + wait 的公共实现（bash 工具与取消测试共用）。
pub(crate) async fn run_shell(
    command: &str,
    workdir: &std::path::Path,
    timeout: Duration,
    cancel: &tokio_util::sync::CancellationToken,
) -> ToolOutput {
    #[cfg(unix)]
    {
        // 进程组：超时/取消时 killpg 全组终止，不留孙进程孤儿（watchdog 先例）。
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c")
            .arg(command)
            .current_dir(workdir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0);
        // tokio 1.40 的 process_group 内部即 pre_exec setpgid；无 unix 之外的
        // 分支（本工具 cfg(unix)）。

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::error(
                    ToolErrorKind::Io(format!("spawn failed: {e}")),
                    format!("failed to spawn bash: {e}"),
                );
            }
        };
        let pid = child.id();
        // 先拆出 stdout/stderr 读取句柄：取消/超时路径 kill 后仍能 drain
        // 子进程已写入的部分（spec 语义：取消不抹掉已产出的输出）。
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_r = stdout.map(tokio::io::BufReader::new);
        let stderr_r = stderr.map(tokio::io::BufReader::new);

        // 常驻 drain task：把管道数据持续推入共享缓冲（不依赖 wait 的完成）。
        let buf: std::sync::Arc<std::sync::Mutex<String>> = Default::default();
        let mut drainers = Vec::new();
        async fn drain<R: tokio::io::AsyncRead + Unpin>(
            mut r: R,
            buf: std::sync::Arc<std::sync::Mutex<String>>,
        ) {
            use tokio::io::AsyncReadExt;
            let mut chunk = [0u8; 8192];
            loop {
                match r.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut b = buf.lock().expect("drain buf poisoned");
                        if b.len() < MAX_CAPTURE_BYTES {
                            b.push_str(&String::from_utf8_lossy(&chunk[..n]));
                        }
                    }
                }
            }
        }
        if let Some(r) = stdout_r {
            let buf2 = buf.clone();
            drainers.push(tokio::spawn(drain(r, buf2)));
        }
        if let Some(r) = stderr_r {
            let buf2 = buf.clone();
            drainers.push(tokio::spawn(drain(r, buf2)));
        }

        let wait_fut = child.wait();
        let cancel_fut = cancel.cancelled();
        let timeout_fut = tokio::time::sleep(timeout);

        let result: ToolOutput = tokio::select! {
            out = wait_fut => {
                match out {
                    Ok(status) => {
                        // 等 drain 收尾（管道 EOF 已到，几乎立即返回）。
                        for d in drainers.drain(..) {
                            let _ = d.await;
                        }
                        let combined = std::mem::take(&mut *buf.lock().expect("drain buf poisoned"));
                        finish_with_status(status, combined)
                    }
                    Err(e) => ToolOutput::error(
                        ToolErrorKind::Io(format!("wait failed: {e}")),
                        format!("waiting for command failed: {e}"),
                    ),
                }
            }
            _ = timeout_fut => {
                kill_group(pid);
                let output = drain_final(&buf, &mut drainers).await;
                ToolOutput {
                    ok: false,
                    // 模型可见的超时说明（tool_result 只回传 output 文本）。
                    output: format!(
                        "{output}\n[command exceeded {}s and was killed]",
                        timeout.as_secs()
                    ),
                    truncated: false,
                    exit_code: None,
                    error: Some(ToolErrorKind::Timeout),
                }
            }
            _ = cancel_fut => {
                kill_group(pid);
                let output = drain_final(&buf, &mut drainers).await;
                ToolOutput {
                    ok: false,
                    output: format!("{output}\n[command cancelled]"),
                    truncated: false,
                    exit_code: None,
                    error: Some(ToolErrorKind::Cancelled),
                }
            }
        };
        result
    }
    #[cfg(not(unix))]
    {
        let _ = (command, workdir, timeout, cancel);
        ToolOutput::error(ToolErrorKind::Io("unix only in 1a".into()), "unsupported platform")
    }
}


/// 扫描 /proc：cmdline 恰为 [`sleep, tag`] 的存活进程（孤儿检测）。
/// tag 唯一化以避免并行测试间的相互干扰。仅测试使用。
#[cfg(test)]
fn find_sleep_proc(tag: &str) -> Option<u32> {
    let dir = std::fs::read_dir("/proc").ok()?;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(pid_str) = name.to_str() else { continue };
        if !pid_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let args: Vec<String> = cmdline
            .split(|&b| b == 0u8)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .filter(|s| !s.is_empty())
            .collect();
        if args == vec!["sleep".to_string(), tag.to_string()] {
            return pid_str.parse::<u32>().ok();
        }
    }
    None
}

#[cfg(unix)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // 负 pid = 进程组 id（子进程 setpgid(0,0) 后其 pgid == pid）。
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        // 竞态兜底：组尚未建立时 kill 单进程。
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
fn finish_with_status(status: std::process::ExitStatus, mut combined: String) -> ToolOutput {
    if status.success() {
        ToolOutput::ok(combined).capped(BASH_OUTPUT_CAP)
    } else {
        // 非零退出 = 数据：ok:true 携带 exit_code，且把退出码写进模型可见文本
        //（tool_result 只回传 output 字符串——没有这行，模型对失败是盲的）。
        let code = status.code();
        if let Some(c) = code {
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str(&format!("[exit code: {c}]"));
        }
        ToolOutput {
            ok: true,
            output: combined,
            truncated: false,
            exit_code: code,
            error: None,
        }
        .capped(BASH_OUTPUT_CAP)
    }
}

/// 取消/超时路径：killpg 之后给管道一个短暂的 EOF 窗口，收集已写入字节。
#[cfg(unix)]
async fn drain_final(
    buf: &std::sync::Arc<std::sync::Mutex<String>>,
    drainers: &mut Vec<tokio::task::JoinHandle<()>>,
) -> String {
    // kill 已发出；子进程短暂存活期间还会继续写管道。给它 100ms。
    for d in drainers.drain(..) {
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), d).await;
    }
    let out = buf.lock().expect("drain buf poisoned").clone();
    crate::message::ToolOutput::ok(out).capped(BASH_OUTPUT_CAP).output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(workdir: &std::path::Path) -> ToolCtx {
        ToolCtx::new(
            workdir.to_path_buf(),
            tokio_util::sync::CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn runs_command_and_returns_output() {
        let dir = tempfile::tempdir().unwrap();
        let out = BashTool::new(Duration::from_secs(5))
            .execute(
                serde_json::json!({"command": "echo hello-bash"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.ok);
        assert!(out.output.contains("hello-bash"), "{}", out.output);
    }

    #[tokio::test]
    async fn nonzero_exit_is_ok_true_with_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let out = BashTool::new(Duration::from_secs(5))
            .execute(
                serde_json::json!({"command": "echo before-fail; exit 3"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.ok, "failure is data, not a tool error");
        assert_eq!(out.exit_code, Some(3));
        assert!(out.output.contains("before-fail"));
        assert!(out.error.is_none());
    }

    #[tokio::test]
    async fn timeout_kills_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let out = BashTool::new(Duration::from_secs(5))
            .execute(
                serde_json::json!({"command": "echo start; sleep 30", "timeout_secs": 1}),
                &ctx(dir.path()),
            )
            .await;
        assert!(!out.ok);
        assert!(matches!(out.error, Some(ToolErrorKind::Timeout)));
    }

    #[tokio::test]
    async fn cancel_terminates_the_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let c2 = cancel.clone();
        let workdir = dir.path().to_path_buf();
        // 用 run_shell 直接驱动（与工具同路径），后台跑长命令。
        let handle = tokio::spawn(async move {
            run_shell("echo spawned; sleep 456123", &workdir, Duration::from_secs(60), &c2).await
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
        let out = handle.await.unwrap();
        assert!(matches!(out.error, Some(ToolErrorKind::Cancelled)));
        assert!(out.output.contains("spawned"), "partial output kept: {}", out.output);

        // 无孤儿：/proc 里不允许还有 cmdline 恰为 ["sleep","30"] 的进程。
        // 用 /proc 扫描而非 `ps` 文本匹配——后者会把测试自身的命令行
        // （bash -c ... sleep 30 ...）误判为孤儿。
        tokio::time::sleep(Duration::from_millis(300)).await;
        let orphan = find_sleep_proc("456123");
        assert!(orphan.is_none(), "orphan survived killpg: {:?}", orphan);
    }

    #[tokio::test]
    async fn missing_command_is_invalid_args() {
        let dir = tempfile::tempdir().unwrap();
        let out = BashTool::new(Duration::from_secs(5))
            .execute(serde_json::json!({}), &ctx(dir.path()))
            .await;
        assert!(!out.ok);
        assert!(matches!(out.error, Some(ToolErrorKind::InvalidArgs)));
    }
}
