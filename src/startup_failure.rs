//! 统一的「启动失败」退出路径（fail-fast-on-startup-errors）。
//!
//! 规约（openspec/changes/fail-fast-on-startup-errors）：任何 sebas 子命令
//! （core/webui/router/run/update/im）在达到 ready 之前发生 fatal 时 SHALL：
//!
//! 1. 以 `startup-failure: <可读原因>` 作为 stderr 的**最后一行**；
//! 2. 当 `SEBAS_STARTUP_ERROR_FILE` 环境变量设置时，把同一行**覆盖写**进该
//!    文件（沙箱验收 / systemd ExecStartPre / 集成测试的机器可读契约）；
//! 3. 以 `EX_TEMPFAIL` (75) 退出——systemd `Restart=on-failure` 看到 75 走
//!    指数退避，不会无限快速重启掩盖根因；与运行期崩溃（既有退出码）区分。
//!
//! 该文件同时是「最近一次启动尝试失败」的闩锁：core 成功达到 ready 后调用
//! [`clear_env_summary_file`] 删除它，这样 channel 客户端（standalone webui）
//! 在 core 不可达时读到的摘要是本次启动尝试的真实结果，不会把陈旧失败
//! 报给已恢复的部署。

/// 启动失败的保留退出码（EX_TEMPFAIL）。与 `crate::watchdog::EXIT_BIND_FAILED`
/// 同值：bind 失败是启动失败的一种，supervisor 据此标记 Degraded。
pub const EXIT_STARTUP_FAILURE: i32 = 75;

/// stderr 末行与错误摘要文件共用的行前缀。
pub const SUMMARY_PREFIX: &str = "startup-failure: ";

/// `SEBAS_STARTUP_ERROR_FILE`：设置了就把摘要行覆盖写入该文件。
pub const SUMMARY_FILE_ENV: &str = "SEBAS_STARTUP_ERROR_FILE";

/// 组装摘要行：`startup-failure: <cause>`。cause 压成单行（换行 → "; "）：
/// stderr「最后一行」契约要求摘要必须独占一行——toml 解析错误等多行原因
/// 否则会把摘要行切碎，触发者读到的末行只是原因的尾巴。
pub fn summary_line(cause: &str) -> String {
    let flat = cause.replace(['\r', '\n'], "; ");
    format!("{SUMMARY_PREFIX}{flat}")
}

/// 打印摘要行到 stderr（约定为该进程 stderr 的最后一行——调用方应先输出
/// 其他诊断信息再调用本函数）。
pub fn eprintln_summary(cause: &str) {
    eprintln!("{}", summary_line(cause));
}

/// 当 `SEBAS_STARTUP_ERROR_FILE` 设置时，把摘要行覆盖写入该文件。
/// 写失败只告警不 panic——stderr 摘要仍是主通道。
pub fn write_env_summary_file(cause: &str) {
    let Some(path) = std::env::var_os(SUMMARY_FILE_ENV) else {
        return;
    };
    if path.is_empty() {
        return;
    }
    if let Err(e) = std::fs::write(std::path::PathBuf::from(&path), summary_line(cause)) {
        eprintln!("warning: cannot write {SUMMARY_FILE_ENV}={}: {e}", path.to_string_lossy());
    }
}

/// 完整的启动失败上报：stderr 末行 + 错误摘要文件（若设置）。
pub fn report(cause: &str) {
    eprintln_summary(cause);
    write_env_summary_file(cause);
}

/// 上报后以 75 退出。仅在「已确定无法达到 ready」的路径调用。
pub fn exit_startup_failure(cause: &str) -> ! {
    report(cause);
    std::process::exit(EXIT_STARTUP_FAILURE);
}

/// 读取最近一次启动失败的摘要原因（`startup-failure: ` 之后的部分）。
/// 文件不存在 / 环境变量未设置 / 无摘要行 → `None`。
///
/// 消费方：core session channel 客户端（standalone webui）在 core 不可达时
/// 把该摘要并进 `reachability.cause`（core-session-channel spec delta）。
pub fn read_env_summary() -> Option<String> {
    let path = std::env::var_os(SUMMARY_FILE_ENV)?;
    if path.is_empty() {
        return None;
    }
    let content = std::fs::read_to_string(std::path::PathBuf::from(&path)).ok()?;
    content
        .lines()
        .rev()
        .find(|l| l.starts_with(SUMMARY_PREFIX))
        .map(|l| l[SUMMARY_PREFIX.len()..].trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 成功达到 ready 后清除摘要闩锁（删除错误文件）。失败静默——文件只是
/// 诊断辅助，不是状态真源。
pub fn clear_env_summary_file() {
    if let Some(path) = std::env::var_os(SUMMARY_FILE_ENV)
        && !path.is_empty()
    {
        let _ = std::fs::remove_file(std::path::PathBuf::from(&path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// env 是进程全局的：并行用例共用会互相污染，用互斥锁串行化
    /// （与 webui_cmd auth_gate_tests 同款模式）。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard {
        saved: Option<std::ffi::OsString>,
    }

    fn set_env_file(path: &std::path::Path) -> EnvGuard {
        // SAFETY: ENV_LOCK 由调用方持有，无并发 env 访问。
        unsafe {
            let saved = std::env::var_os(SUMMARY_FILE_ENV);
            std::env::set_var(SUMMARY_FILE_ENV, path);
            EnvGuard { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: 同上，ENV_LOCK 仍被持有。
            unsafe {
                match self.saved.take() {
                    Some(v) => std::env::set_var(SUMMARY_FILE_ENV, v),
                    None => std::env::remove_var(SUMMARY_FILE_ENV),
                }
            }
        }
    }

    #[test]
    fn summary_line_has_the_canonical_prefix() {
        assert_eq!(
            summary_line("config error: bad toml"),
            "startup-failure: config error: bad toml"
        );
    }

    #[test]
    fn env_file_gets_overwritten_with_the_summary() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("startup-error.log");
        // 预置陈旧内容：覆盖写语义必须清掉它。
        std::fs::write(&path, "startup-failure: stale from last time").unwrap();

        let _guard = set_env_file(&path);
        write_env_summary_file("config error: garbage");
        drop(_guard);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "startup-failure: config error: garbage");
    }

    #[test]
    fn env_file_unset_is_a_noop() {
        let _env = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK 已持有。
        unsafe { std::env::remove_var(SUMMARY_FILE_ENV) };
        // 不 panic 即为通过（stderr 副作用无断言面）。
        write_env_summary_file("whatever");
    }

    #[test]
    fn read_env_summary_returns_cause_after_prefix() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("startup-error.log");
        std::fs::write(
            &path,
            "other log line\nstartup-failure: bind failed: addr in use\n",
        )
        .unwrap();
        let _guard = set_env_file(&path);
        assert_eq!(
            read_env_summary(),
            Some("bind failed: addr in use".into())
        );
        drop(_guard);
    }

    #[test]
    fn read_env_summary_missing_file_or_env_is_none() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // 文件不存在 → None。
        let _guard = set_env_file(&dir.path().join("nope.log"));
        assert_eq!(read_env_summary(), None);
        drop(_guard);
        // 文件存在但无摘要行 → None。
        std::fs::write(dir.path().join("plain.log"), "hello\n").unwrap();
        let _guard = set_env_file(&dir.path().join("plain.log"));
        assert_eq!(read_env_summary(), None);
        drop(_guard);
        // 环境变量未设置 → None。
        unsafe { std::env::remove_var(SUMMARY_FILE_ENV) };
        assert_eq!(read_env_summary(), None);
    }

    #[test]
    fn clear_removes_the_latch_file() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("startup-error.log");
        std::fs::write(&path, "startup-failure: old").unwrap();
        let _guard = set_env_file(&path);
        clear_env_summary_file();
        drop(_guard);
        assert!(!path.exists(), "ready 后闩锁文件必须被清除");
        assert_eq!(read_env_summary(), None);
    }
}
