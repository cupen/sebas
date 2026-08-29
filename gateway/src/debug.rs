//! Debug 模式注入路径。
//!
//! `enable_debug_test_provider` 仅在 `sebas gateway --debug` 与
//! `sebas run --debug` 路径调用，不属于生产配置语义。把它从 `config.rs`
//! 挪出，避免后续维护者疑惑为什么 prod config 解析里混着 debug 注入逻辑。
//!
//! 注入内容：provider 名 `test`、base_url `gateway://self` 哨兵值（proxy
//! 短路应答，不实际拨号）；并把 `test → test` 路由插到路由表最前。详见
//! `crate::test_provider`（响应生成）与 `crate::config::resolve_api_keys`
//! （跳过 test provider 的 api_key 校验）。

use std::collections::HashMap;

use crate::config::{GatewayConfig, ProviderConfig, RouteGroup};

/// `--debug`：配置解析完成后注入内置 test provider + 优先路由。
/// 幂等：重复调用不产生重复路由（已有则跳过）。
///
/// 拆出来作为模块顶层函数而不是 `GatewayConfig` 的方法：
/// - 强调这是「注入行为」而非 config 解析语义的一部分；
/// - 给后续可能的扩展（如 debug-time mock upstream fixture）留独立 seam；
/// - 要求 prod config.rs 不混 debug 注入代码。
pub fn enable_debug_test_provider(cfg: &mut GatewayConfig) {
    cfg.debug = true;
    tracing::debug!("debug mode: injecting built-in test provider");
    cfg.providers.insert(
        "test".to_string(),
        ProviderConfig {
            base_url_anthropic: Some("gateway://self".to_string()),
            base_url_openai: Some("gateway://self".to_string()),
            api_key_env: None,
            api_key: None,
            model_map: HashMap::new(),
            models: Vec::new(),
        },
    );
    if !cfg
        .routes
        .iter()
        .any(|r| r.model == "test" && r.providers.len() == 1 && r.providers[0] == "test")
    {
        cfg.routes.insert(
            0,
            RouteGroup {
                model: "test".to_string(),
                providers: vec!["test".to_string()],
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_isolated(raw: &str) -> crate::error::Result<GatewayConfig> {
        // SAFETY: 测试串行持有 LOCK（见 config.rs 同名 helper 的注释），
        // 设一次就清，避免污染本进程内其他测试。
        unsafe {
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                "__sebas_test_no_overlay__.json",
            );
        }
        GatewayConfig::parse(raw)
    }

    // 跨模块共享锁（crate::test_util::CONFIG_ENV_LOCK 的别名）——config.rs 与本
    // 模块的测试都动 SEBAS_GATEWAY_LISTEN，必须互斥。
    static LOCK: &std::sync::Mutex<()> = &crate::test_util::CONFIG_ENV_LOCK;

    #[test]
    fn enable_debug_test_provider_injects_test_provider_and_route() {
        let _g = LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_LISTEN");
        }
        let raw = r#"
[gateway]
auth_token = "sk-test"
[provider.anthropic]
api_key = "test-key"
"#;
        let mut cfg = parse_isolated(raw).expect("parse");
        assert!(!cfg.debug);
        enable_debug_test_provider(&mut cfg);

        assert!(cfg.debug);
        let test = cfg.providers.get("test").expect("test provider injected");
        assert_eq!(test.base_url_anthropic.as_deref(), Some("gateway://self"));
        assert_eq!(test.api_key, None);
        assert_eq!(test.api_key_env, None);
        assert!(
            cfg.routes
                .iter()
                .any(|r| r.model == "test" && r.providers.len() == 1 && r.providers[0] == "test"),
            "test → test route must be injected"
        );
        assert_eq!(
            cfg.routes[0].model, "test",
            "debug route should lead the table"
        );

        // resolve_api_keys 跳过内置 test provider（不触达上游，无需 key）。
        let keys = cfg.resolve_api_keys().expect("resolve_api_keys");
        assert!(!keys.contains_key("test"));
        assert!(keys.contains_key("anthropic"));

        // 幂等：重复注入不产生重复路由。
        enable_debug_test_provider(&mut cfg);
        assert_eq!(
            cfg.routes
                .iter()
                .filter(|r| r.model == "test" && r.providers.len() == 1 && r.providers[0] == "test")
                .count(),
            1
        );
    }
}
