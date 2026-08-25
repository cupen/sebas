//! ServiceManager：受管服务句柄表 + 期望状态三层合成 + persist 落盘。
//!
//! 三层（design.md D5）：config 默认 → `~/.sebas/services.json` 覆盖 →
//! 运行时 ServiceSet 覆盖。监督本体在 supervisor.rs 的每服务 task；
//! 本模块负责聚合查询与期望态翻译。

use crate::config::WatchdogWebUiConfig;
use crate::watchdog::control::DesiredState;
use crate::watchdog::supervisor::{
    ServiceCommand, ServiceHandle, ServiceName, ServiceSnapshot, ServiceSpec, ServiceState,
};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use tracing::warn;

// ─── 保留的 webui endpoint 助手 ────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiEndpoint {
    pub host: String,
    pub port: u16,
}

impl WebUiEndpoint {
    pub fn from_config(config: &WatchdogWebUiConfig) -> Option<Self> {
        config.enabled.then(|| Self {
            host: config.host.clone(),
            port: config.port,
        })
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn is_loopback(&self) -> bool {
        self.host
            .parse::<IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false)
    }
}

pub fn should_start_watchdog_webui(config: &WatchdogWebUiConfig) -> bool {
    config.enabled
}

// ─── ServiceManager ────────────────────────────────────────

/// persist 文件里单服务的期望态字符串。
fn desired_to_str(d: DesiredState) -> &'static str {
    match d {
        DesiredState::Enabled => "on",
        DesiredState::Disabled => "off",
    }
}

fn desired_from_str(s: &str) -> Option<DesiredState> {
    match s {
        "on" | "enabled" => Some(DesiredState::Enabled),
        "off" | "disabled" => Some(DesiredState::Disabled),
        _ => None,
    }
}

struct ManagedEntry {
    handle: ServiceHandle,
    /// config 层初值：false 且无任何覆盖时，快照报告 `Disabled`。
    config_enabled: bool,
    /// persist 覆盖层（启动时从 services.json 读入；persist 写入时更新）。
    file_desired: Option<DesiredState>,
}

/// 受管服务表。clone 共享同一批监督 task。
#[derive(Clone)]
pub struct ServiceManager {
    services: std::sync::Arc<std::sync::Mutex<HashMap<ServiceName, ManagedEntry>>>,
    persist_path: PathBuf,
}

/// set_desired / restart 的失败原因（面向 control 面的错误消息）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceOpError {
    UnknownService(String),
    PersistWrite(String),
}

impl std::fmt::Display for ServiceOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceOpError::UnknownService(name) => write!(f, "未知服务: {name}"),
            ServiceOpError::PersistWrite(e) => write!(f, "写 services.json 失败: {e}"),
        }
    }
}

/// 三层合成的初始期望态（纯函数）：file 覆盖 config。
pub fn initial_desired(config_enabled: bool, file: Option<DesiredState>) -> DesiredState {
    file.unwrap_or(match config_enabled {
        true => DesiredState::Enabled,
        false => DesiredState::Disabled,
    })
}

impl ServiceManager {
    pub fn new(persist_path: PathBuf) -> Self {
        Self {
            services: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            persist_path,
        }
    }

    /// 读 persist 文件，返回 per-service 覆盖。文件缺失/损坏 → 空表（不阻断启动）。
    pub fn read_persisted(path: &PathBuf) -> HashMap<ServiceName, DesiredState> {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return HashMap::new();
        };
        let Ok(table) = serde_json::from_str::<HashMap<String, String>>(&raw) else {
            warn!("services.json 解析失败，忽略: {}", path.display());
            return HashMap::new();
        };
        table
            .into_iter()
            .filter_map(|(k, v)| Some((service_from_str(&k)?, desired_from_str(&v)?)))
            .collect()
    }

    fn write_persisted(&self, map: &HashMap<ServiceName, DesiredState>) -> Result<(), String> {
        let table: HashMap<&str, &str> = map
            .iter()
            .map(|(k, v)| (k.as_str(), desired_to_str(*v)))
            .collect();
        if let Some(parent) = self.persist_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(&table).map_err(|e| e.to_string())?;
        std::fs::write(&self.persist_path, json).map_err(|e| e.to_string())
    }

    /// 注册并启动一个服务的监督 task。`config_enabled` 为 config 层初值。
    pub fn register(&self, mut spec: ServiceSpec, config_enabled: bool) {
        let name = spec.name;
        // 三层合成（此时只有前两层：file 覆盖 config；运行时层由命令改变）。
        let file_map = Self::read_persisted(&self.persist_path);
        let file_desired = file_map.get(&name).copied();
        spec.desired = initial_desired(config_enabled, file_desired);
        let (handle, _task) = crate::watchdog::supervisor::start_supervision(spec);
        self.services.lock().unwrap().insert(
            name,
            ManagedEntry {
                handle,
                config_enabled,
                file_desired,
            },
        );
    }

    fn entry(&self, name: ServiceName) -> Option<ServiceHandle> {
        self.services
            .lock()
            .unwrap()
            .get(&name)
            .map(|e| e.handle.clone())
    }

    /// 设置期望态；`persist: true` 时同步写 services.json。
    pub async fn set_desired(
        &self,
        name: ServiceName,
        desired: DesiredState,
        persist: bool,
    ) -> Result<(), ServiceOpError> {
        let handle = self
            .entry(name)
            .ok_or_else(|| ServiceOpError::UnknownService(name.as_str().into()))?;
        if persist {
            let mut map = Self::read_persisted(&self.persist_path);
            map.insert(name, desired);
            self.write_persisted(&map)
                .map_err(ServiceOpError::PersistWrite)?;
            if let Some(e) = self.services.lock().unwrap().get_mut(&name) {
                e.file_desired = Some(desired);
            }
        }
        let cmd = match desired {
            DesiredState::Enabled => ServiceCommand::Start,
            DesiredState::Disabled => ServiceCommand::Stop,
        };
        let _ = handle.send(cmd).await;
        Ok(())
    }

    /// 立即重启（ServiceRestart）。core 传 `is_upgrade` 标记新二进制。
    pub async fn restart(&self, name: ServiceName, is_upgrade: bool) -> Result<(), ServiceOpError> {
        let handle = self
            .entry(name)
            .ok_or_else(|| ServiceOpError::UnknownService(name.as_str().into()))?;
        let _ = handle.send(ServiceCommand::Restart { is_upgrade }).await;
        Ok(())
    }

    /// core 升级完成后的重启（PostAction 语义）。
    pub async fn restart_core_after_upgrade(&self) {
        let _ = self.restart(ServiceName::Core, true).await;
    }

    /// 单服务快照：supervisor 的 Stopped + config 关 + 无覆盖 → Disabled。
    pub async fn snapshot(&self, name: ServiceName) -> Option<ServiceSnapshot> {
        let (handle, config_enabled, file_desired) = {
            let services = self.services.lock().unwrap();
            let e = services.get(&name)?;
            (e.handle.clone(), e.config_enabled, e.file_desired)
        };
        let mut snap = handle.snapshot().await;
        if snap.state == ServiceState::Stopped
            && !config_enabled
            && file_desired.is_none()
            && snap.desired == DesiredState::Disabled
        {
            snap.state = ServiceState::Disabled;
        }
        Some(snap)
    }

    /// 全部服务快照（固定顺序 core → webui → gateway）。
    pub async fn all_snapshots(&self) -> Vec<ServiceSnapshot> {
        let mut out = Vec::new();
        for name in [ServiceName::Core, ServiceName::WebUi, ServiceName::Gateway] {
            if let Some(snap) = self.snapshot(name).await {
                out.push(snap);
            }
        }
        out
    }

    /// watchdog 退出：停全部 child 并结束监督 task。
    ///
    /// 发完 Shutdown 命令后**等待各服务真正进入 Stopped/Disabled**（上限
    /// 10s）：只 await 发送就返回的话，run_watchdog 随即退出、runtime drop
    /// 取消监督 task，kill_on_drop 的 SIGKILL 会与 child.stop() 的 SIGTERM
    /// 竞速——core 可能没机会走优雅关闭（会话状态快照落盘）。超时兜底
    /// 放行，SIGKILL backstop 仍由 kill_on_drop 承担。
    pub async fn shutdown_all(&self) {
        let handles: Vec<ServiceHandle> = self
            .services
            .lock()
            .unwrap()
            .values()
            .map(|e| e.handle.clone())
            .collect();
        for h in handles {
            let _ = h.send(ServiceCommand::Shutdown).await;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let settled = self
                .all_snapshots()
                .await
                .into_iter()
                .all(|s| matches!(s.state, ServiceState::Stopped | ServiceState::Disabled));
            if settled || std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    pub fn persist_path(&self) -> &PathBuf {
        &self.persist_path
    }
}

/// 服务名字符串解析（RPC `service` 字段 / services.json key 共用）。
pub fn service_from_str(s: &str) -> Option<ServiceName> {
    match s {
        "core" => Some(ServiceName::Core),
        "webui" => Some(ServiceName::WebUi),
        "gateway" => Some(ServiceName::Gateway),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SebasError;
    use crate::watchdog::supervisor::{ServiceSpawner, SpawnedInstance};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// 永不自动退出的 Fake spawner（监督 task 静置）。
    struct NopSpawner {
        spawns: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ServiceSpawner for NopSpawner {
        async fn spawn(&self) -> crate::error::Result<SpawnedInstance> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = tokio::sync::watch::channel(false);
            Ok(SpawnedInstance {
                child: Box::new(NopChild {
                    pid: 1000,
                    exited: rx,
                    exit_tx: tx,
                }),
                readiness: None,
            })
        }
    }

    struct NopChild {
        pid: u32,
        exited: tokio::sync::watch::Receiver<bool>,
        exit_tx: tokio::sync::watch::Sender<bool>,
    }

    #[async_trait::async_trait]
    impl crate::watchdog::supervisor::ManagedChild for NopChild {
        fn pid(&self) -> Option<u32> {
            Some(self.pid)
        }
        async fn wait(&mut self) -> Option<i32> {
            let mut exited = self.exited.clone();
            while !*exited.borrow() {
                if exited.changed().await.is_err() {
                    return None;
                }
            }
            Some(0)
        }
        async fn stop(&mut self) {
            let _ = self.exit_tx.send(true);
        }
    }

    fn fast_spec(name: ServiceName) -> ServiceSpec {
        let mut spec = ServiceSpec::new(
            name,
            std::sync::Arc::new(NopSpawner {
                spawns: AtomicUsize::new(0),
            }),
            DesiredState::Enabled,
        );
        spec.crash = crate::watchdog::supervisor::CrashPolicy::new(
            Duration::from_millis(50),
            3,
            Duration::from_millis(5),
            Duration::from_millis(20),
        );
        spec.restart_delay = Duration::from_millis(5);
        spec.spawn_retry_delay = Duration::from_millis(5);
        spec
    }

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sebas-svc-{}-{}.json", tag, std::process::id()))
    }

    // ── 纯函数：三层合成 ──

    #[test]
    fn initial_desired_file_overrides_config() {
        use DesiredState::{Disabled, Enabled};
        // config on，file off → off。
        assert_eq!(initial_desired(true, Some(Disabled)), Disabled);
        // config off，file on → on（config 只是初值）。
        assert_eq!(initial_desired(false, Some(Enabled)), Enabled);
        // 无 file 覆盖 → config。
        assert_eq!(initial_desired(true, None), Enabled);
        assert_eq!(initial_desired(false, None), Disabled);
    }

    // ── persist 读写 ──

    #[test]
    fn persisted_file_roundtrip() {
        let path = tmp_path("roundtrip");
        let _ = std::fs::remove_file(&path);
        let mgr = ServiceManager::new(path.clone());
        {
            let mut map = HashMap::new();
            map.insert(ServiceName::WebUi, DesiredState::Disabled);
            mgr.write_persisted(&map).unwrap();
        }
        let read = ServiceManager::read_persisted(&path);
        assert_eq!(read.get(&ServiceName::WebUi), Some(&DesiredState::Disabled));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_or_broken_persist_file_is_empty() {
        let path = tmp_path("broken");
        let _ = std::fs::remove_file(&path);
        assert!(ServiceManager::read_persisted(&path).is_empty());
        std::fs::write(&path, "not json{").unwrap();
        assert!(ServiceManager::read_persisted(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    // ── 快照 Disabled 语义 + persist 写/不写 ──

    #[tokio::test]
    async fn config_off_without_override_reports_disabled() {
        let path = tmp_path("disabled");
        let _ = std::fs::remove_file(&path);
        let mgr = ServiceManager::new(path.clone());
        mgr.register(fast_spec(ServiceName::Gateway), false);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let snap = mgr.snapshot(ServiceName::Gateway).await.unwrap();
        assert_eq!(
            snap.state,
            ServiceState::Disabled,
            "config 关+无覆盖 → Disabled"
        );
        mgr.shutdown_all().await;
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn set_desired_persist_false_skips_file() {
        let path = tmp_path("nopersist");
        let _ = std::fs::remove_file(&path);
        let mgr = ServiceManager::new(path.clone());
        mgr.register(fast_spec(ServiceName::WebUi), true);
        tokio::time::sleep(Duration::from_millis(10)).await;

        mgr.set_desired(ServiceName::WebUi, DesiredState::Disabled, false)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!path.exists(), "persist=false 不得写 services.json");
        let snap = mgr.snapshot(ServiceName::WebUi).await.unwrap();
        assert_eq!(snap.state, ServiceState::Stopped);

        mgr.shutdown_all().await;
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn set_desired_persist_true_writes_file_and_survives_restart() {
        let path = tmp_path("persist");
        let _ = std::fs::remove_file(&path);
        let mgr = ServiceManager::new(path.clone());
        mgr.register(fast_spec(ServiceName::Gateway), true);
        tokio::time::sleep(Duration::from_millis(10)).await;

        mgr.set_desired(ServiceName::Gateway, DesiredState::Disabled, true)
            .await
            .unwrap();
        assert!(path.exists(), "persist=true 必须写 services.json");
        mgr.shutdown_all().await;

        // 「重启 watchdog」：新 manager 读同一文件，gateway 期望态应为 off。
        let read = ServiceManager::read_persisted(&path);
        assert_eq!(
            read.get(&ServiceName::Gateway),
            Some(&DesiredState::Disabled)
        );
        assert_eq!(
            initial_desired(true, read.get(&ServiceName::Gateway).copied()),
            DesiredState::Disabled
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn unknown_service_rejected() {
        let path = tmp_path("unknown");
        let _ = std::fs::remove_file(&path);
        let mgr = ServiceManager::new(path);
        let err = mgr
            .set_desired(ServiceName::Gateway, DesiredState::Disabled, false)
            .await
            .unwrap_err();
        assert_eq!(err, ServiceOpError::UnknownService("gateway".into()));
    }

    // 保留的 webui 助手测试。

    #[test]
    fn watchdog_starts_webui_task_from_config() {
        let raw = r#"
[feishu]
app_id = "a"
app_secret = "b"

[watchdog.webui]
enabled = true
host = "127.0.0.1"
port = 9798
"#;
        let cfg = crate::config::Config::parse(raw).expect("config parses");
        assert!(should_start_watchdog_webui(&cfg.watchdog.webui));
        assert_eq!(
            WebUiEndpoint::from_config(&cfg.watchdog.webui),
            Some(WebUiEndpoint {
                host: "127.0.0.1".into(),
                port: 9798,
            })
        );
    }

    #[test]
    fn service_name_roundtrip() {
        for name in ["core", "webui", "gateway"] {
            let parsed = service_from_str(name).unwrap();
            assert_eq!(parsed.as_str(), name);
        }
        assert!(service_from_str("feishu").is_none());
        assert!(service_from_str("").is_none());
    }

    // 占位使用 SebasError import（NopSpawner 返回类型别名保持简洁）。
    #[allow(dead_code)]
    fn _unused(_: SebasError) {}
}
