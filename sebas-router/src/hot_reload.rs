//! 热重载状态面（retire-legacy-state-json 3.5 起：文件监视已删）。
//!
//! 机制：provider/alias 的热重载由 core 状态通道驱动
//! （`crate::core_channel::subscribe_once` 收帧 → `reload_from_channel` →
//! `apply_overlay_value` → `swap_core`）。本模块**不做任何 I/O**，只持有
//! reload 结果状态供 `/admin/stats` 读取：
//!
//! - `last_error`：最近一次 reload 校验失败的错误文本（成功时清空）；
//! - `last_ok_at`：最近一次成功 reload 的时间；
//! - `source_unavailable`：数据源（core 通道）不可用的成因，与 reload 失败
//!   区分——前者是通道断连，后者可能是配置校验问题。
//!
//! 曾经这里还有一个 `notify` 文件监视器（watch providers.json 所在目录 +
//! 300ms debounce + mtime 轮询兜底），随 overlay 文件读取一并退休：该文件
//! 已无写入方，监视一个永不变化的文件没有意义（design D6）。

use std::sync::{Arc, RwLock};
use std::time::SystemTime;

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
}