//! env 覆盖链：env > TOML > 默认。
//! 独立成文件 = 独立进程：set_var/remove_var 不会与其他 config 测试的
//! 并行断言竞争（cargo 在单进程内以线程并行跑同一文件的测试）。

use sebas::config::Config;

#[test]
fn env_overrides_toml_and_can_satisfy_required_fields() {
    // SAFETY: 本进程只有这一个测试线程读写这些变量。
    unsafe {
        std::env::set_var("SEBAS_FEISHU_APP_ID", "cli_env");
        std::env::set_var("SEBAS_FEISHU_APP_SECRET", "sec_env");
        std::env::set_var("SEBAS_LOG_LEVEL", "trace");
    }
    let r1 = Config::parse(
        r#"
[feishu]
app_id = "cli_toml"
app_secret = "sec_toml"

[log]
level = "debug"
"#,
    );
    // env 还可补齐 TOML 缺失的必填字段（无文件纯 env 部署路径）。
    let r2 = Config::parse("[feishu]\n");
    unsafe {
        std::env::remove_var("SEBAS_FEISHU_APP_ID");
        std::env::remove_var("SEBAS_FEISHU_APP_SECRET");
        std::env::remove_var("SEBAS_LOG_LEVEL");
    }

    let cfg = r1.expect("parse with env overrides");
    assert_eq!(cfg.feishu.app_id, "cli_env", "env beats TOML");
    assert_eq!(cfg.feishu.app_secret, "sec_env", "env beats TOML");
    assert_eq!(cfg.log.level, "trace", "SEBAS_LOG_LEVEL beats TOML");

    let cfg2 = r2.expect("env satisfies required fields");
    assert_eq!(cfg2.feishu.app_id, "cli_env");
    assert_eq!(cfg2.feishu.app_secret, "sec_env");

    // 变量移除后，`[feishu]` 空表 = 双双留空 = 不接入飞书（sebas-2ty 起
    // feishu 可选），解析应成功且 app_id 为空（确认测试间无残留污染）。
    let r3 = Config::parse("[feishu]\n").expect("feishu 可选：双空合法");
    assert_eq!(r3.feishu.app_id, "");
    assert_eq!(r3.feishu.app_secret, "");

    // 半配置（只填其一）仍报错。
    let r4 = Config::parse("[feishu]\napp_id = \"only_id\"\n");
    assert!(r4.is_err());
}
