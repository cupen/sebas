//! 上游 provider api key 解析扩展点（spec 2026-08-17 §2.15）。
//!
//! 之前 `GatewayConfig::resolve_api_keys` 直接内联三种来源（env 变量 / 明文 /
//! 无），没给 vault / 1Password / SSH agent / KMS 等后端留 seam。本模块
//! 抽出 `KeyResolver` trait + 默认 env-based impl，未来要接其他密钥后端
//! 只需新增一个实现，不动调用方。
//!
//! 调用方语义不变：`resolve_api_keys` 把每个 provider 的「key 来源」折成
//! `KeyHint` 喂给 resolver，resolver 返回 `Result<String, String>`。
//! 错误信息只含变量名 / provider 名，绝不含 key 值。

use std::sync::atomic::{AtomicBool, Ordering};

/// key 来源提示 —— 三种当前支持的形式 + 留位给未来 `KeyResolver` 实现
/// 解释自定义 hint。
///
/// - `EnvVar(name)`：从进程 env 读 `name` 对应的值（首选，密钥不落盘）。
/// - `Plain(s)`：明文内联（仅测试用；resolve 时 warn 一次）。
/// - `None`：provider 没配任何 key → 解析必失败。
///
/// 未来 vault / KMS 等后端可以加 `KeyHint::Vault { path }` /
/// `KeyHint::OnePassword { ref }` 等变体；新 impl 自行决定如何处理。
#[derive(Debug, Clone)]
pub enum KeyHint {
    /// 从环境变量读。优先项 —— 密钥永远不落 config 文件。
    EnvVar(String),
    /// 明文内联 key。仅测试用；resolver 命中此分支会 emit 一次 warn。
    Plain(String),
    /// 没配置 key —— resolver 必返回 Err。
    None,
}

/// 上游 api key 解析器。
///
/// `Send + Sync`：gateway server 多线程（axum + tokio runtime），resolver
/// 可能被多个 worker 同时调用；用 `Arc<dyn KeyResolver>` 在 `AppState`
/// 里共享。
///
/// 错误信息约束：必须只含变量名 / provider 名 / 错误类别，**绝不能含
/// key 值**（哪怕是部分）。调用方负责把 `Err(String)` 升级到
/// `GatewayError::Config` 并保留 provider name。
pub trait KeyResolver: Send + Sync {
    fn resolve(&self, hint: &KeyHint) -> Result<String, String>;
}

/// 默认 impl：从进程 env 读 `EnvVar`；`Plain` 直接返回（emit 一次全局
/// warn）；`None` → 错。
///
/// warn-once：用一个进程级 `AtomicBool` 守门，确保即便 resolver 被反复
/// 调用（未来 per-request 实现可能复用同一个 Arc<dyn KeyResolver>），
/// 明文 api_key 警告也只打一次。
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvKeyResolver;

static PLAIN_WARN_EMITTED: AtomicBool = AtomicBool::new(false);

impl KeyResolver for EnvKeyResolver {
    fn resolve(&self, hint: &KeyHint) -> Result<String, String> {
        match hint {
            KeyHint::EnvVar(name) => match std::env::var(name) {
                Ok(v) if !v.is_empty() => Ok(v),
                _ => Err(format!(
                    "api_key_env 指向的环境变量 '{name}' 未设置或为空"
                )),
            },
            KeyHint::Plain(_) => {
                // 进程级 warn-once：resolve 可能被多次调用（启动 + 热重载 +
                // 未来 per-request impl），明文 key 警告只打一次不刷屏。
                if !PLAIN_WARN_EMITTED.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "gateway provider 使用明文 api_key（config 内联或 /provider overlay 写入；如需更严格的密钥管理请改用 api_key_env）"
                    );
                }
                // 安全：match 已限定 Plain(s) 分支，这里取出 inner String。
                if let KeyHint::Plain(s) = hint {
                    Ok(s.clone())
                } else {
                    unreachable!("match arm mismatch")
                }
            }
            KeyHint::None => Err("provider 未配置 api_key_env 或 api_key".to_string()),
        }
    }
}

/// 测试用 stub：忽略 hint，永远返回固定 key。
///
/// 主要给未来 `resolve_api_keys` 接受 `&dyn KeyResolver` 参数时的单测用
/// —— 不污染真实 env、不触发 warn。
#[derive(Debug, Clone, Copy)]
pub struct StubKeyResolver {
    pub fixed: &'static str,
}

impl KeyResolver for StubKeyResolver {
    fn resolve(&self, _hint: &KeyHint) -> Result<String, String> {
        Ok(self.fixed.to_string())
    }
}

/// 测试 helper：从 `ProviderConfig` 派生 `KeyHint`，集中 plain / env 的
/// 优先级判断。和 `config.rs::resolve_providers` 里 `api_key_env` vs
/// `api_key` 的优先级保持一致（plain 优先于 env）。
///
/// 拆成 helper 而不是在 `resolve_api_keys` 里内联：方便后续 `VaultKeyResolver`
/// 等实现独立测试 hint 派生逻辑，不必绕过 `GatewayConfig::parse`。
pub fn hint_from_provider(
    api_key: Option<&str>,
    api_key_env: Option<&str>,
) -> KeyHint {
    if let Some(plain) = api_key.filter(|s| !s.is_empty()) {
        KeyHint::Plain(plain.to_string())
    } else if let Some(name) = api_key_env.filter(|s| !s.is_empty()) {
        KeyHint::EnvVar(name.to_string())
    } else {
        KeyHint::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // 串行化所有改动进程 env 的测试 —— `std::env::set_var` 是进程级全局，
    // 并行跑会撞到。`gateway::config::tests::LOCK` 的同款惯例。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 重置全局 warn-once 状态：每个 `Plain` 测试都应看到自己的第一次
    /// 调用 emit warn；之后清掉让下一个测试独立。当前实现是「once per
    /// process」，测试间需要手动重置（用 `#[test]` 内部 + helper）。
    fn reset_plain_warn() {
        PLAIN_WARN_EMITTED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn env_var_set_returns_value() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("SEBAS_KEY_RESOLVER_TEST_A", "sk-from-env");
        }
        let hint = KeyHint::EnvVar("SEBAS_KEY_RESOLVER_TEST_A".into());
        let r = EnvKeyResolver.resolve(&hint);
        assert_eq!(r.as_deref(), Ok("sk-from-env"));
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("SEBAS_KEY_RESOLVER_TEST_A");
        }
    }

    #[test]
    fn env_var_unset_errors() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("SEBAS_KEY_RESOLVER_TEST_B");
        }
        let hint = KeyHint::EnvVar("SEBAS_KEY_RESOLVER_TEST_B".into());
        let r = EnvKeyResolver.resolve(&hint);
        let err = r.expect_err("unset env var must error");
        assert!(
            err.contains("SEBAS_KEY_RESOLVER_TEST_B"),
            "error must name the env var, never the value: {err}"
        );
        assert!(
            !err.contains("sk-"),
            "error must never leak a key value: {err}"
        );
    }

    #[test]
    fn env_var_empty_string_errors() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::set_var("SEBAS_KEY_RESOLVER_TEST_C", "");
        }
        let hint = KeyHint::EnvVar("SEBAS_KEY_RESOLVER_TEST_C".into());
        let r = EnvKeyResolver.resolve(&hint);
        let err = r.expect_err("empty env var must error");
        assert!(err.contains("SEBAS_KEY_RESOLVER_TEST_C"));
        // SAFETY: ENV_LOCK held.
        unsafe {
            std::env::remove_var("SEBAS_KEY_RESOLVER_TEST_C");
        }
    }

    #[test]
    fn plain_returns_value() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_plain_warn();
        let hint = KeyHint::Plain("sk-plain-test".into());
        let r = EnvKeyResolver.resolve(&hint);
        assert_eq!(r.as_deref(), Ok("sk-plain-test"));
    }

    #[test]
    fn none_errors_with_no_key_configured_message() {
        let _g = ENV_LOCK.lock().unwrap();
        let hint = KeyHint::None;
        let r = EnvKeyResolver.resolve(&hint);
        let err = r.expect_err("None must error");
        assert!(
            err.contains("未配置") || err.contains("api_key"),
            "error should explain that no key was configured: {err}"
        );
    }

    #[test]
    fn plain_warn_fires_only_once_across_calls() {
        let _g = ENV_LOCK.lock().unwrap();
        reset_plain_warn();
        // 第一次调用应触发 warn-once 路径；通过 `PLAIN_WARN_EMITTED` 的
        // 状态间接验证（直接断言 tracing 输出需要 subscriber，太重）。
        assert!(!PLAIN_WARN_EMITTED.load(Ordering::Relaxed));
        let _ = EnvKeyResolver.resolve(&KeyHint::Plain("sk-a".into()));
        assert!(
            PLAIN_WARN_EMITTED.load(Ordering::Relaxed),
            "first Plain call must flip the warn-once flag"
        );
        let _ = EnvKeyResolver.resolve(&KeyHint::Plain("sk-b".into()));
        // 第二次仍能正常返回（不会因为 flag 跳到错误分支）。
        assert_eq!(
            EnvKeyResolver
                .resolve(&KeyHint::Plain("sk-c".into()))
                .as_deref(),
            Ok("sk-c")
        );
    }

    #[test]
    fn stub_returns_fixed_value() {
        let r = StubKeyResolver { fixed: "sk-stub" };
        // 忽略 hint 类型，全部返回 fixed。
        assert_eq!(
            r.resolve(&KeyHint::EnvVar("ANY".into())).as_deref(),
            Ok("sk-stub")
        );
        assert_eq!(
            r.resolve(&KeyHint::Plain("anything".into())).as_deref(),
            Ok("sk-stub")
        );
        assert_eq!(
            r.resolve(&KeyHint::None).as_deref(),
            Ok("sk-stub"),
            "stub intentionally ignores None — it's a test double"
        );
    }

    #[test]
    fn hint_from_provider_prefers_plain_over_env() {
        let h = hint_from_provider(Some("sk-plain"), Some("ENV_NAME"));
        assert!(
            matches!(h, KeyHint::Plain(ref s) if s == "sk-plain"),
            "plain must win over env when both set: {h:?}"
        );
    }

    #[test]
    fn hint_from_provider_env_only() {
        let h = hint_from_provider(None, Some("ENV_NAME"));
        assert!(
            matches!(h, KeyHint::EnvVar(ref n) if n == "ENV_NAME"),
            "env-only must produce EnvVar: {h:?}"
        );
    }

    #[test]
    fn hint_from_provider_none_when_both_missing() {
        let h = hint_from_provider(None, None);
        assert!(matches!(h, KeyHint::None));
    }

    #[test]
    fn hint_from_provider_empty_strings_treated_as_missing() {
        // 和 `GatewayConfig::resolve_providers` 一致：空字符串 ≡ 缺省。
        let h = hint_from_provider(Some(""), Some(""));
        assert!(matches!(h, KeyHint::None));
        let h = hint_from_provider(Some(""), Some("ENV"));
        assert!(matches!(h, KeyHint::EnvVar(ref n) if n == "ENV"));
    }
}
