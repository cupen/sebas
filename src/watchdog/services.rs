//! ServiceManager：受管服务句柄表 + 期望状态三层合成 + persist 落盘。
//!
//! 三层（design.md D6）：config 默认 → 状态目录派生的 services.json
//! （`SEBAS_SERVICES_FILE` 可覆盖）→ 运行时 ServiceSet 覆盖。监督本体在
//! supervisor.rs 的每服务 task；本模块负责聚合查询与期望态翻译。

use crate::config::ServiceWebUiConfig;
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
    pub fn from_config(config: &ServiceWebUiConfig) -> Option<Self> {
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
            warn!(
                "failed to parse services.json, ignoring: {}",
                path.display()
            );
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

    /// 注册恒启动服务（core，enable-core-by-default）：忽略 config 层与
    /// services.json 覆盖层，期望态恒为 Enabled。历史 `core: off` 覆盖会打
    /// 一条 deprecation warn 后被忽略。
    pub fn register_core(&self, mut spec: ServiceSpec) {
        debug_assert_eq!(spec.name, ServiceName::Core);
        let name = spec.name;
        if Self::read_persisted(&self.persist_path).contains_key(&name) {
            warn!(
                "services.json has an override for 'core' but core is always started; override ignored"
            );
        }
        spec.desired = DesiredState::Enabled;
        let (handle, _task) = crate::watchdog::supervisor::start_supervision(spec);
        self.services.lock().unwrap().insert(
            name,
            ManagedEntry {
                handle,
                config_enabled: true,
                file_desired: None,
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

    /// 直接记录一条启动失败（fail-fast-on-startup-errors：rollback 失败等
    /// 发生在监督 task 之外的路径）。写进该服务的共享快照，`sebas ctl
    /// status` 随即可见；服务状态置 `FailedStartup` 终态。
    pub async fn record_startup_failure(&self, name: ServiceName, cause: &str) {
        let Some(handle) = self.entry(name) else {
            return;
        };
        let shared = handle.shared_snapshot();
        let mut snap = shared.lock().await;
        snap.state = ServiceState::FailedStartup;
        snap.startup_failure = Some(crate::watchdog::supervisor::StartupFailureInfo {
            count: 1,
            last_stderr: cause.to_string(),
            at_unix: crate::watchdog::supervisor::now_unix_secs(),
        });
    }

    /// 全部服务快照（固定顺序 core → webui → router → im；status-driven-
    /// service-rows：ServiceStatus 列表恰为受管 entry 快照集合，im 也是受管
    /// entry——缺了它「im managed when enabled」就永远不含真实 im 行）。
    pub async fn all_snapshots(&self) -> Vec<ServiceSnapshot> {
        let mut out = Vec::new();
        for name in [
            ServiceName::Core,
            ServiceName::WebUi,
            ServiceName::Router,
            ServiceName::Im,
        ] {
            if let Some(snap) = self.snapshot(name).await {
                out.push(snap);
            }
        }
        out
    }

    /// watchdog exited：停全部 child 并结束监督 task。
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
            let settled = self.all_snapshots().await.into_iter().all(|s| {
                matches!(
                    s.state,
                    ServiceState::Stopped
                        | ServiceState::Disabled
                        | ServiceState::Degraded
                        | ServiceState::FailedStartup
                )
            });
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
        "router" => Some(ServiceName::Router),
        "im" => Some(ServiceName::Im),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SebasError;
    use crate::watchdog::supervisor::{ServiceSpawner, SpawnedInstance};
    use std::path::Path;
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
        mgr.register(fast_spec(ServiceName::Router), false);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let snap = mgr.snapshot(ServiceName::Router).await.unwrap();
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
        mgr.register(fast_spec(ServiceName::Router), true);
        tokio::time::sleep(Duration::from_millis(10)).await;

        mgr.set_desired(ServiceName::Router, DesiredState::Disabled, true)
            .await
            .unwrap();
        assert!(path.exists(), "persist=true 必须写 services.json");
        mgr.shutdown_all().await;

        // 「重启 watchdog」：新 manager 读同一文件，router 期望态应为 off。
        let read = ServiceManager::read_persisted(&path);
        assert_eq!(
            read.get(&ServiceName::Router),
            Some(&DesiredState::Disabled)
        );
        assert_eq!(
            initial_desired(true, read.get(&ServiceName::Router).copied()),
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
            .set_desired(ServiceName::Router, DesiredState::Disabled, false)
            .await
            .unwrap_err();
        assert_eq!(err, ServiceOpError::UnknownService("router".into()));
    }

    #[tokio::test]
    async fn register_core_ignores_persisted_off_override() {
        // enable-core-by-default：services.json 里的历史 `core: off` 覆盖被
        // 忽略——core 期望态恒 Enabled，不会被持久化层停用。
        let path = tmp_path("core-always-on");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, r#"{"core":"off"}"#).unwrap();

        let mgr = ServiceManager::new(path.clone());
        mgr.register_core(fast_spec(ServiceName::Core));
        tokio::time::sleep(Duration::from_millis(10)).await;

        let snap = mgr.snapshot(ServiceName::Core).await.unwrap();
        assert_eq!(snap.desired, DesiredState::Enabled, "core 期望态恒 Enabled");
        assert_ne!(
            snap.state,
            ServiceState::Disabled,
            "core 不得因 services.json 覆盖而停用"
        );
        mgr.shutdown_all().await;
        let _ = std::fs::remove_file(&path);
    }

    // 保留的 webui 助手测试。

    #[test]
    fn watchdog_starts_webui_task_from_config() {
        let raw = r#"
[feishu]
app_id = "a"
app_secret = "b"

[service.webui]
enabled = true
host = "127.0.0.1"
port = 9798
"#;
        let cfg = crate::config::Config::parse(raw).expect("config parses");
        assert!(cfg.service.webui.enabled);
        assert_eq!(
            WebUiEndpoint::from_config(&cfg.service.webui),
            Some(WebUiEndpoint {
                host: "127.0.0.1".into(),
                port: 9798,
            })
        );
    }

    #[test]
    fn service_name_roundtrip() {
        for name in ["core", "webui", "router"] {
            let parsed = service_from_str(name).unwrap();
            assert_eq!(parsed.as_str(), name);
        }
        assert!(service_from_str("feishu").is_none());
        assert!(service_from_str("").is_none());
    }

    // ── single-state-dir 5.1/5.2：落点收编 + 越界回归 ──

    /// 本组用例的进程级 env 串行锁（动 HOME / SEBAS_STATE_DIR 全局变量）。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// env 钉住 + 保存/恢复护栏。
    struct PinnedEnv {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl PinnedEnv {
        /// 状态目录 = pin，操作员主目录 = fake_home，无逐文件覆盖。
        fn pin(pin: &Path, fake_home: &Path) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let vars: &[&'static str] = &[
                "SEBAS_STATE_DIR",
                "SEBAS_HOME",
                "HOME",
                "SEBAS_SERVICES_FILE",
            ];
            let saved = vars.iter().map(|v| (*v, std::env::var_os(v))).collect();
            unsafe {
                std::env::set_var("SEBAS_STATE_DIR", pin);
                std::env::set_var("HOME", fake_home);
                std::env::remove_var("SEBAS_HOME");
                std::env::remove_var("SEBAS_SERVICES_FILE");
            }
            Self { saved, _lock: lock }
        }
    }

    impl Drop for PinnedEnv {
        fn drop(&mut self) {
            for (v, prev) in &self.saved {
                match prev {
                    Some(val) => unsafe { std::env::set_var(v, val) },
                    None => unsafe { std::env::remove_var(v) },
                }
            }
        }
    }

    /// 目录清单 + mtime 快照（越界比对用）。
    fn dir_footprint(dir: &Path) -> Vec<(String, Option<std::time::SystemTime>)> {
        fn walk(dir: &Path, out: &mut Vec<(String, Option<std::time::SystemTime>)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let mtime = e.metadata().ok().and_then(|m| m.modified().ok());
                out.push((name, mtime));
                if e.path().is_dir() {
                    walk(&e.path(), out);
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, &mut out);
        out.sort();
        out
    }

    /// 5.2 越界回归：钉住状态目录并完整跑一次覆盖层生命周期——注册（读
    /// 覆盖层）→ 覆盖层写入（ServiceSet 的落盘动作）→ 管理器销毁重建（
    /// watchdog 重启）→ 覆盖层读回。全程 fake 操作员主目录（`HOME` 指向
    /// 一次性目录）**未被创建或修改**（清单 + mtime 存在性比对），落点在
    /// 钉住的目录内。
    #[tokio::test]
    async fn pinned_lifecycle_never_touches_operator_home() {
        let pin_dir = tempfile::tempdir().unwrap();
        let fake_home = tempfile::tempdir().unwrap();
        let _env = PinnedEnv::pin(pin_dir.path(), fake_home.path());

        let operator_sebas = fake_home.path().join(".sebas");
        let before = dir_footprint(fake_home.path());

        // ── 启动：watchdog 以派生落点建管理器（run_watchdog 的装配形态）。
        let persist = crate::watchdog::services_persist_path_for_test();
        assert_eq!(persist, pin_dir.path().join("services.json"));
        let mgr = ServiceManager::new(persist.clone());
        mgr.register(fast_spec(ServiceName::WebUi), false);
        tokio::time::sleep(Duration::from_millis(10)).await;

        // ── 覆盖层写入（ServiceSet persist=true 的终点是 watchdog 自己）。
        mgr.set_desired(ServiceName::WebUi, DesiredState::Enabled, true)
            .await
            .unwrap();
        assert!(
            persist.exists(),
            "覆盖层必须落在钉住的目录内: {}",
            persist.display()
        );
        mgr.shutdown_all().await;

        // ── 停止/重启：新管理器从文件读回覆盖（重启后仍生效）。
        let read = ServiceManager::read_persisted(&persist);
        assert_eq!(
            read.get(&ServiceName::WebUi),
            Some(&DesiredState::Enabled),
            "runtime 覆盖在 watchdog 重启后仍生效"
        );

        // ── 越界比对：操作员主目录未被创建/修改。
        assert!(
            !operator_sebas.exists(),
            "操作员真实配置目录不得被创建: {}",
            operator_sebas.display()
        );
        assert_eq!(
            dir_footprint(fake_home.path()),
            before,
            "fake 操作员主目录的清单与 mtime 逐项未变"
        );
    }

    /// 5.1 验收的读写行为半边：落点收编后，覆盖层的读取/写入行为与改造前
    /// 完全一致（三层合成 + ServiceSet 落盘语义不变）。
    #[tokio::test]
    async fn derived_persist_path_keeps_read_write_semantics() {
        let pin_dir = tempfile::tempdir().unwrap();
        let fake_home = tempfile::tempdir().unwrap();
        let _env = PinnedEnv::pin(pin_dir.path(), fake_home.path());

        let persist = crate::watchdog::services_persist_path_for_test();
        let mgr = ServiceManager::new(persist.clone());
        // config 层关（webui 默认不启用）：无覆盖时快照 = config 层初值。
        mgr.register(fast_spec(ServiceName::WebUi), false);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let snap = mgr.snapshot(ServiceName::WebUi).await.unwrap();
        assert_eq!(snap.state, ServiceState::Disabled, "无覆盖 → config 层初值");

        // ServiceSet 落盘：文件内容形状不变（{"webui":"on"}）。
        mgr.set_desired(ServiceName::WebUi, DesiredState::Enabled, true)
            .await
            .unwrap();
        let raw = std::fs::read_to_string(&persist).unwrap();
        assert!(raw.contains("webui") && raw.contains("on"), "{raw}");
        let read = ServiceManager::read_persisted(&persist);
        assert_eq!(
            initial_desired(false, read.get(&ServiceName::WebUi).copied()),
            DesiredState::Enabled,
            "file 覆盖层翻掉 config 初值——三层合成语义不变"
        );

        mgr.shutdown_all().await;
    }

    // 占位使用 SebasError import（NopSpawner 返回类型别名保持简洁）。
    #[allow(dead_code)]
    fn _unused(_: SebasError) {}
}
