//! providers.json 热重载（Task 4.1/4.2）。
//!
//! watch overlay 所在**目录**（tempfile+rename 换 inode，直接 watch 文件
//! 在 Linux inotify 上会掉事件），按路径过滤 + 300ms debounce 后走与
//! admin 写同一条 reload 管线（`reload_and_swap`）。admin 自写用
//! `AtomicBool` 抑制一拍（同一变更只 reload 一次）。notify 初始化失败
//! （容器/受限 fs）降级 2s mtime 轮询。
//!
//! reload 校验失败保旧内核 + warn + `last_reload_error` 记入 admin 状态
//! （供 stats）；下次有效写自动恢复（4.2）。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use notify::Watcher;
use crate::server::AppState;

/// 最近一次 reload 失败的错误文本（None = 无失败/成功）。挂进 AppState
/// 供 `/admin/stats` 读取；成功 reload 时清空。
#[derive(Default)]
pub struct ReloadStatus {
    last_error: RwLock<Option<String>>,
    last_ok_at: RwLock<Option<SystemTime>>,
    /// 数据源（core state channel）不可用的成因（5.3）。与 `last_error`
    /// 区分：reload 失败可能是配置校验问题，数据源不可用是通道断连。
    /// 通道恢复时清空。
    source_unavailable: RwLock<Option<String>>,
    /// admin 写路径记录：最近一次 admin 成功写入后的 providers.json 内容
    /// hash。watcher 触发时若文件 hash 仍等于它 → 该事件来自 admin 自写
    /// （admin 已 reload），跳过；否则是外部写，需要 reload。
    /// 用内容 hash 而非计数抑制位：N 次 admin 写在 debounce 里合并成
    /// K < N 次事件时，计数位会残留并误吞下一次真正的外部写（e2e 复现过）。
    admin_write_hash: Mutex<Option<u64>>,
}

impl ReloadStatus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn error(&self) -> Option<String> {
        self.last_error.read().ok().and_then(|g| g.clone())
    }

    pub fn ok_at(&self) -> Option<SystemTime> {
        *self.last_ok_at.read().ok()?
    }

    /// 数据源不可用的成因（None = 通道健康）。
    pub fn source_unavailable(&self) -> Option<String> {
        self.source_unavailable
            .read()
            .ok()
            .and_then(|g| g.clone())
    }

    /// 记录数据源不可用（5.3 断连）。
    pub(crate) fn record_source_unavailable(&self, cause: &str) {
        if let Ok(mut g) = self.source_unavailable.write() {
            *g = Some(cause.to_string());
        }
    }

    /// 数据源恢复（通道重连成功）。
    pub(crate) fn record_source_ok(&self) {
        if let Ok(mut g) = self.source_unavailable.write() {
            *g = None;
        }
    }

    pub(crate) fn record_ok_quiet(&self) {
        self.record_ok();
    }

    pub(crate) fn record_err(&self, e: &str) {
        self.record_err_inner(e);
    }

    fn record_ok(&self) {
        if let Ok(mut g) = self.last_error.write() {
            *g = None;
        }
        if let Ok(mut g) = self.last_ok_at.write() {
            *g = Some(SystemTime::now());
        }
    }

    fn record_err_inner(&self, e: &str) {
        if let Ok(mut g) = self.last_error.write() {
            *g = Some(e.to_string());
        }
    }

    /// 记录 admin 成功写入后的文件内容（hash）。watcher 事件到达时比对。
    pub(crate) fn mark_admin_write(&self, content: &str) {
        let mut h = DefaultHasher::new();
        content.hash(&mut h);
        if let Ok(mut g) = self.admin_write_hash.lock() {
            *g = Some(h.finish());
        }
    }

    /// 判断本次 watcher 事件是否由 admin 自写产生（内容未变 → 是）。
    /// 比对后清除记录，避免残留。
    pub(crate) fn is_admin_write(&self, content: &str) -> bool {
        let mut h = DefaultHasher::new();
        content.hash(&mut h);
        match self.admin_write_hash.lock() {
            Ok(mut g) => g.take() == Some(h.finish()),
            Err(_) => false,
        }
    }
}

/// 外部写入触发的 reload 管线：走 admin 同款 `reload_and_swap`（重读 toml
/// 种子 + overlay → swap_core，失败保旧内核）。admin 版自带 status 记录，
/// 这里只需区分日志文案。
pub fn reload(state: &AppState, _status: &ReloadStatus) -> Result<(), String> {
    match crate::admin::reload_and_swap(state) {
        Ok(()) => {
            tracing::info!("providers.json 热重载成功");
            Ok(())
        }
        Err(e) => {
            tracing::warn!("providers.json 热重载失败（保旧内核继续服务）: {e}");
            Err(e)
        }
    }
}

/// 起 watcher（tokio task）。overlay 路径取自启动期内核 cfg（overlay 路径
/// 本身不热变）。task 存活于进程生命周期。返回的 oneshot 在 notify watch
/// 注册完成后 resolve——测试用它同步「watcher 就绪」再写文件。
pub fn spawn_watcher(state: AppState, status: Arc<ReloadStatus>) -> tokio::sync::oneshot::Receiver<()> {
    let path = PathBuf::from(&state.core().cfg.provider_overlay);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        watch_loop(state, status, path, ready_tx).await;
    });
    ready_rx
}

async fn watch_loop(
    state: AppState,
    status: Arc<ReloadStatus>,
    path: PathBuf,
    ready: tokio::sync::oneshot::Sender<()>,
) {
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    // notify 回调非 async：经 std channel + 专用转发线程投递到 tokio mpsc。
    let (std_tx, std_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(move |res| {
        let _ = std_tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("notify 不可用，降级 2s mtime 轮询: {e}");
            let _ = ready.send(());
            return poll_loop(state, status, path).await;
        }
    };
    if let Err(e) = watcher.watch(&dir, notify::RecursiveMode::NonRecursive) {
        tracing::warn!("notify watch {dir:?} 失败，降级 2s mtime 轮询: {e}");
        let _ = ready.send(());
        return poll_loop(state, status, path).await;
    }
    let _ = ready.send(());
    let name = path.file_name().map(std::ffi::OsStr::to_owned);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(16);
    // 转发线程持有 watcher（watch 须持续存活）：std channel → tokio mpsc，
    // 只投「路径命中本文件」的信号。
    std::thread::spawn(move || {
        for res in std_rx {
            if event_hits(&res, name.as_deref())
                && tx.blocking_send(()).is_err()
            {
                break;
            }
        }
        drop(watcher);
    });

    loop {
        // debounce：收到事件后等 300ms 静默再 reload；期间新事件重置窗口。
        match rx.recv().await {
            Some(()) => {
                // 静默窗口。
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(300)) => break,
                        Some(()) = rx.recv() => {}
                        else => break,
                    }
                }
                maybe_reload(&state, &status, &path);
            }
            None => {
                // 转发线程退出（进程收尾）。
                return;
            }
        }
    }
}

fn event_hits(res: &notify::Result<notify::Event>, name: Option<&std::ffi::OsStr>) -> bool {
    let Ok(ev) = res else { return false };
    let Some(name) = name else { return false };
    ev.paths.iter().any(|p| p.file_name() == Some(name))
}

/// mtime 轮询兜底（notify 不可用）。
async fn poll_loop(state: AppState, status: Arc<ReloadStatus>, path: PathBuf) {
    let mut last_mtime = file_mtime(&path);
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let m = file_mtime(&path);
        if m != last_mtime {
            last_mtime = m;
            maybe_reload(&state, &status, &path);
        }
    }
}

/// 事件/轮询触发的 reload 入口：admin 自写（内容 hash 未变）跳过。
fn maybe_reload(state: &AppState, status: &ReloadStatus, path: &Path) {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    if status.is_admin_write(&content) {
        return;
    }
    let _ = reload(state, status);
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::Watcher;

    #[test]
    fn notify_events_carry_file_name() {
        let dir = std::env::temp_dir().join(format!("sebas-hr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("providers.json");
        std::fs::write(&file, "{}").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut w = notify::recommended_watcher(move |r| {
            let _ = tx.send(r);
        })
        .unwrap();
        w.watch(&dir, notify::RecursiveMode::NonRecursive).unwrap();
        std::fs::write(&file, "{\"a\":1}").unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let mut hit = false;
        for r in rx.try_iter() {
            if event_hits(&r, file.file_name()) {
                hit = true;
            }
        }
        assert!(hit, "写同路径文件须产生可命中事件");
        drop(w);
        std::fs::remove_dir_all(&dir).ok();
    }
}
