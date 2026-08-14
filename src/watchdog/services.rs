use crate::config::WatchdogWebUiConfig;
use std::net::IpAddr;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebUiOwner {
    None,
    LegacyRun,
    Watchdog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebUiGuardError {
    AlreadyOwned { owner: WebUiOwner },
    PortConflict { bind_addr: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUiLifecycle {
    owner: WebUiOwner,
    endpoint: Option<WebUiEndpoint>,
    core_generation: u64,
}

impl Default for WebUiLifecycle {
    fn default() -> Self {
        Self {
            owner: WebUiOwner::None,
            endpoint: None,
            core_generation: 0,
        }
    }
}

impl WebUiLifecycle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn owner(&self) -> WebUiOwner {
        self.owner
    }

    pub fn endpoint(&self) -> Option<&WebUiEndpoint> {
        self.endpoint.as_ref()
    }

    pub fn core_generation(&self) -> u64 {
        self.core_generation
    }

    pub fn start_watchdog_owned(
        &mut self,
        endpoint: WebUiEndpoint,
    ) -> std::result::Result<(), WebUiGuardError> {
        self.start(WebUiOwner::Watchdog, endpoint)
    }

    pub fn start_legacy_run(
        &mut self,
        endpoint: WebUiEndpoint,
    ) -> std::result::Result<(), WebUiGuardError> {
        self.start(WebUiOwner::LegacyRun, endpoint)
    }

    fn start(
        &mut self,
        owner: WebUiOwner,
        endpoint: WebUiEndpoint,
    ) -> std::result::Result<(), WebUiGuardError> {
        if self.owner != WebUiOwner::None {
            return Err(WebUiGuardError::AlreadyOwned { owner: self.owner });
        }
        if endpoint.port == 0 {
            return Err(WebUiGuardError::PortConflict {
                bind_addr: endpoint.bind_addr(),
            });
        }
        self.owner = owner;
        self.endpoint = Some(endpoint);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.owner = WebUiOwner::None;
        self.endpoint = None;
    }

    pub fn record_core_restart(&mut self) {
        self.core_generation = self.core_generation.saturating_add(1);
    }
}

pub fn should_start_watchdog_webui(config: &WatchdogWebUiConfig) -> bool {
    config.enabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

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
        let cfg = Config::parse(raw).expect("config parses");
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
    fn core_restart_does_not_stop_watchdog_webui() {
        let mut lifecycle = WebUiLifecycle::new();
        let endpoint = WebUiEndpoint {
            host: "127.0.0.1".into(),
            port: 9797,
        };
        lifecycle.start_watchdog_owned(endpoint.clone()).unwrap();

        lifecycle.record_core_restart();

        assert_eq!(lifecycle.owner(), WebUiOwner::Watchdog);
        assert_eq!(lifecycle.endpoint(), Some(&endpoint));
        assert_eq!(lifecycle.core_generation(), 1);
    }

    #[test]
    fn double_start_guarded() {
        let mut lifecycle = WebUiLifecycle::new();
        let endpoint = WebUiEndpoint {
            host: "127.0.0.1".into(),
            port: 9797,
        };
        lifecycle.start_watchdog_owned(endpoint.clone()).unwrap();

        let err = lifecycle.start_legacy_run(endpoint).unwrap_err();

        assert_eq!(
            err,
            WebUiGuardError::AlreadyOwned {
                owner: WebUiOwner::Watchdog,
            }
        );
    }
}
