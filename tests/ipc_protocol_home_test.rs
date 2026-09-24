//! 协议之家纪律的机械断言（unify-ipc-protocol-home 1.2）。
//!
//! 把「`sebas-ipc` 是每个角色都够得着的协议之家，且不含任何角色实现」从
//! 约定变成 CI 可机械核对的事实（spec `ipc-protocol-home`
//! 「The protocol crate depends only on neutral leaves」）：
//!
//! 1. **依赖图**：`cargo tree -p sebas-ipc` 内不得出现任何角色实现
//!    （根 `sebas` / webui / router / im）与执行节点 `sebas-node`；
//!    `sebas-domain` / `sebas-channels` 这类中立叶子允许出现。
//! 2. **公开面**：`sebas-ipc/src/` 不得出现角色 crate 引用，也不得出现任何
//!    **域表名**字面量（协议 crate 不认识持久层）。
//!
//! 名字一律按**完整包名**匹配：`sebas-node-link` 是两侧共用的中立链路契约
//! crate，拿子串 `sebas-node` 匹配会误伤它。
//!
//! 失败演示（1.2 要求，已实做）：给 `sebas-ipc/Cargo.toml` 临时加一行
//! `sebas-webui = { path = "../sebas-webui" }` → `sebas_ipc_depends_on_no_role_implementation`
//! 立刻红（见 `tasks.md` 的证据行与 `evidence/` 记录）。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 主控角色实现 + 执行节点（协议之家的依赖图里一律禁见）。
const FORBIDDEN_PACKAGES: &[&str] = &[
    "sebas",
    "sebas-webui",
    "sebas-router",
    "sebas-im",
    "sebas-node",
];

/// 公开源码里禁见的角色 crate 引用。注意 `sebas_node_link` 不在此列：
/// 它是中立链路契约，本就允许被协议侧取用（只是今天没人用）。
const FORBIDDEN_CRATE_REFS: &[&str] = &[
    "sebas_dispatch",
    "sebas_webui",
    "sebas_router",
    "sebas_im",
    "sebas_feishu",
    "sebas_node::",
    "sebas_models",
    "sebas_state",
];

/// 域表名字面量（带引号形态）：协议 crate 不得携带任何持久层知识。
const DOMAIN_TABLE_LITERALS: &[&str] = &[
    "\"projects\"",
    "\"session_map\"",
    "\"model_aliases\"",
    "\"users\"",
    "\"usage_records\"",
    "\"schema_meta\"",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ipc_src_dir() -> PathBuf {
    workspace_root().join("sebas-ipc").join("src")
}

fn rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries = std::fs::read_dir(&d).expect("read dir");
        while let Some(entry) = entries.next().transpose().expect("dir entry") {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// 运行 `cargo tree -p <pkg>`，返回每个依赖的完整包名（不含版本后缀）。
/// 首行是被查询 crate 自己，调用方按需忽略。
fn dep_names(pkg: &str) -> Vec<String> {
    let out = Command::new(env!("CARGO"))
        .args(["tree", "-p", pkg, "--prefix", "none"])
        .current_dir(workspace_root())
        .output()
        .expect("cargo tree 必须可执行（本测试在 workspace 根环境运行）");
    assert!(
        out.status.success(),
        "cargo tree -p {pkg} 失败:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect()
}

/// 1.1/1.2 验收：`sebas-ipc` 的依赖图里没有任何角色实现（含执行节点）。
#[test]
fn sebas_ipc_depends_on_no_role_implementation() {
    let names = dep_names("sebas-ipc");
    let offenders: Vec<&str> = FORBIDDEN_PACKAGES
        .iter()
        .copied()
        .filter(|f| names.iter().any(|n| n == *f))
        .collect();
    assert!(
        offenders.is_empty(),
        "sebas-ipc 的依赖图出现了主控角色/执行节点实现 {offenders:?}；\
         协议之家必须是所有角色都够得着的中立 crate（ipc-protocol-home spec\
         「The protocol crate depends only on neutral leaves」/ design D3：\
         引用角色类型会形成依赖环）。允许的依赖：tokio / interprocess / \
         serde / serde_json / tracing / sebas-domain / sebas-channels。"
    );
}

/// 正向对照：中立叶子确实在树里（避免断言因路径/包名写错而空过），而且
/// 中立的 `sebas-node-link` **不会**被 `sebas-node` 的完整包名匹配误伤。
#[test]
fn neutral_leaves_are_present_and_node_link_is_not_mistaken_for_the_node() {
    let names = dep_names("sebas-ipc");
    assert!(
        names.iter().any(|n| n == "sebas-domain"),
        "sebas-ipc 应依赖中立域层 sebas-domain"
    );
    assert!(
        names.iter().any(|n| n == "sebas-channels"),
        "sebas-ipc 应依赖中立渠道层 sebas-channels"
    );
    assert!(
        !names.iter().any(|n| n == "sebas-node-link"),
        "sebas-ipc 今天并不依赖 node-link（若将来依赖，它也是中立契约，本测试\
         只防角色实现——勿改成子串匹配）"
    );
}

/// 可达性正向对照（spec「Every role can depend on the protocol crate」）：
/// **今天说这条通道的角色**（根 crate 的 core / webui / im 面 + router）都经
/// 普通 path 依赖取用协议之家。
///
/// 注意断言的准确口径：spec 要的是「够得着」（路径依赖存在、且协议 crate
/// 不构成环），不是「每个角色今天都必须依赖它」——`sebas-node` 说的是**节点
/// 链路**（`sebas-node-link`），与 core session channel 无关，强行要求它依赖
/// sebas-ipc 反而是错的。可达性的机械保证由上面两条断言给出：协议之家的
/// 依赖图里没有任何角色实现（因此任何角色加这条依赖都不会成环）。
#[test]
fn the_channel_speakers_reach_the_protocol_crate_by_path_dependency() {
    for pkg in ["sebas", "sebas-router"] {
        let names = dep_names(pkg);
        assert!(
            names.iter().any(|n| n == "sebas-ipc"),
            "{pkg} 的直接依赖里没有 sebas-ipc；协议类型必须经协议之家取用，\
             不允许另立副本（ipc-protocol-home spec「reachable from every role」）"
        );
    }
}

/// 1.2 验收（公开面）：`sebas-ipc/src` 里没有角色 crate 引用，也没有域表名。
#[test]
fn sebas_ipc_public_surface_names_no_role_crate_and_no_domain_table() {
    let files = rs_files(&ipc_src_dir());
    assert!(!files.is_empty(), "sebas-ipc/src 必须有源码");

    for file in files {
        let rel = file.strip_prefix(workspace_root()).unwrap();
        let src = std::fs::read_to_string(&file).expect("read source");
        for crate_ref in FORBIDDEN_CRATE_REFS {
            assert!(
                !src.contains(crate_ref),
                "{} 引用了角色 crate `{crate_ref}`（协议之家不得含角色实现）",
                rel.display()
            );
        }
        for table in DOMAIN_TABLE_LITERALS {
            assert!(
                !src.contains(table),
                "{} 出现域表名字面量 {table}（协议之家不认识持久层）",
                rel.display()
            );
        }
    }
}

/// 1.2 验收（协议类型只有一份家）：根 crate 的 `core_channel/protocol.rs`
/// 不再**定义**任何 wire 类型，只剩原位 `pub use`——「同一协议被声明两遍」
/// 在结构上不再可能（spec「No role carries a private copy of a protocol」）。
#[test]
fn root_crate_re_exports_the_protocol_instead_of_declaring_it() {
    let path = workspace_root().join("src").join("core_channel").join("protocol.rs");
    let src = std::fs::read_to_string(&path).expect("read root protocol.rs");
    for needle in [
        "pub enum CoreChannelRequest",
        "pub enum CoreChannelResponse",
        "pub enum SessionStreamFrame",
        "pub enum StateStreamFrame",
        "pub enum NodeLinkOp",
        "pub enum NodeLinkOutcome",
        "pub struct ChannelHandshake",
        "pub struct Attachment",
    ] {
        assert!(
            !src.contains(needle),
            "根 crate 的 core_channel/protocol.rs 仍在定义 `{needle}`——类型\
             必须唯一定义在 sebas-ipc::protocol，根 crate 只做原位 pub use"
        );
    }
    assert!(
        src.contains("pub use sebas_ipc::protocol::"),
        "根 crate 应原位再导出 sebas_ipc::protocol（调用点零改动）"
    );
}

/// 4.1 验收（router 不再自备协议）：`sebas-router/src` 里不得再出现本地
/// 帧子集声明、本地响应结构或手搓的 `json!` 握手 / 请求。
#[test]
fn router_carries_no_private_copy_of_the_channel_protocol() {
    let files = rs_files(&workspace_root().join("sebas-router").join("src"));
    assert!(!files.is_empty(), "sebas-router/src 必须有源码");
    for file in files {
        let rel = file.strip_prefix(workspace_root()).unwrap();
        let src = std::fs::read_to_string(&file).expect("read source");
        // 注释里**允许**提到这些名字（本 change 的说明性注释就在提它们），
        // 因此剥掉注释行再扫代码。
        let code_only: String = src
            .lines()
            .map(|line| match line.find("//") {
                Some(i) => &line[..i],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n");
        // 注释剥离不覆盖块注释以外的 `//!`/`///`——两者都以 `//` 开头，已被覆盖。
        for needle in [
            "enum StateStreamFrame",
            "struct SnapshotResp",
            "json!({\"secret\"",
            "json!({\"cmd\"",
        ] {
            assert!(
                !code_only.contains(needle),
                "{} 的代码里仍有自备协议痕迹 `{needle}`；router 必须复用\
                 sebas-ipc 的共享定义（spec「The router speaks the channel \
                 through shared types」）",
                rel.display()
            );
        }
    }
}