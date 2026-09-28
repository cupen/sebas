//! 机械断言（add-about-build-info tasks 3.1）：两处 webui 装配点必须把根
//! crate 编译期可见的构建信息接进 `sebas_webui::BuildInfo`。
//!
//! 漏接一个装配点不会让任何行为测试变红——About 只会如实显示 unknown（D3
//! 兜底），属静默回归，只有重跑 5.1 沙箱 HTTP 核对才会发现。这里把 tasks 3.1
//! 的一次性人工 grep 固化为常驻闸门，钉三件事：
//! - `src/webui_cmd.rs`（`sebas webui`）与 `src/run.rs`（`sebas core --webui`）
//!   都构造 `sebas_webui::BuildInfo`；
//! - `version` 一律取 `env!("CARGO_PKG_VERSION")`（与 `sebas --version` 同源
//!   ——clap derive 的 `--version` 也读这同一个变量）；
//! - build_time / git 字段一律经 `crate::upgrade::*` 助手（option_env! 兜底
//!   unknown），不得在装配点自拼来源。

use std::fs;
use std::path::Path;

fn assert_assembly_wires_build_info(rel: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取 {rel} 失败: {e}"));
    assert!(
        src.contains("sebas_webui::BuildInfo {"),
        "{rel} 未构造 sebas_webui::BuildInfo——该装配点 serve 的 /api/about 会退化为\
         全 unknown（add-about-build-info 3.1：每个生产 webui 装配点必须传入构建信息）"
    );
    assert!(
        src.contains(r#"version: env!("CARGO_PKG_VERSION")"#),
        "{rel} 的 BuildInfo.version 未取 env!(\"CARGO_PKG_VERSION\")——version 必须与\
         `sebas --version` 同源（clap derive 同读该变量）"
    );
    for helper in ["build_time", "git_branch", "git_hash"] {
        assert!(
            src.contains(&format!("crate::upgrade::{helper}()")),
            "{rel} 的 BuildInfo.{helper} 未走 crate::upgrade::{helper}()——构建信息只能\
             经 upgrade.rs 助手（option_env! 兜底 unknown），不得在装配点自拼来源"
        );
    }
}

#[test]
fn webui_cmd_assembly_wires_build_info() {
    assert_assembly_wires_build_info("src/webui_cmd.rs");
}

#[test]
fn core_run_assembly_wires_build_info() {
    assert_assembly_wires_build_info("src/run.rs");
}
