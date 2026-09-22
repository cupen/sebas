//! 叶子属性机械断言（add-domain-layer 4.3）。
//!
//! 解析 `cargo tree` 输出，把「共享域层是角色中立的叶子」从约定变成 CI 可
//! 机械核对的事实（openspec/changes/add-domain-layer — design「Risks」）：
//!
//! - `cargo tree -p sebas-domain`：依赖图内不得出现任何主控角色实现
//!   （根 `sebas` / webui / router / im）与执行节点 `sebas-node`；
//! - `cargo tree -p sebas-node`：执行节点依赖纪律（add-remote-execution-node
//!   D0，`execution-node` spec「Execution node process persona」）——不得
//!   出现主控角色实现。
//!
//! 注意名字按**完整包名**匹配：`sebas-node-link` 是两侧共用的中立链路契约
//! crate，允许出现（不能拿子串 `sebas-node` 做匹配）。

use std::process::Command;

/// 主控角色实现 + 执行节点（域层依赖图中一律禁见）。
const FORBIDDEN_FOR_DOMAIN: &[&str] = &[
    "sebas",
    "sebas-webui",
    "sebas-router",
    "sebas-im",
    "sebas-node",
];

/// 执行节点禁见主控角色（根 `sebas` 是 core 角色的宿主）。
const FORBIDDEN_FOR_NODE: &[&str] = &["sebas", "sebas-webui", "sebas-router", "sebas-im"];

/// 运行 `cargo tree -p <pkg>`，返回每个依赖的完整包名（不含版本后缀）。
/// 首行是被查询 crate 自己，调用方按需忽略。
fn dep_names(pkg: &str) -> Vec<String> {
    let out = Command::new("cargo")
        .args(["tree", "-p", pkg, "--prefix", "none", "--depth", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree 必须可执行（本测试在 workspace 根环境运行）");
    assert!(
        out.status.success(),
        "cargo tree -p {pkg} 失败:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let name = line.split_whitespace().next()?;
            Some(name.to_string())
        })
        .collect()
}

#[test]
fn sebas_domain_depends_on_no_role_implementation() {
    let names = dep_names("sebas-domain");
    let offenders: Vec<&str> = FORBIDDEN_FOR_DOMAIN
        .iter()
        .copied()
        .filter(|f| names.iter().any(|n| n == *f))
        .collect();
    assert!(
        offenders.is_empty(),
        "sebas-domain 的依赖图出现了主控角色/执行节点实现 {offenders:?}；\
         共享域层必须是角色中立的叶子（add-domain-layer spec「Neutral leaf \
         dependency」）。允许的依赖：serde / serde_json / serde（派生）/ \
         sebas-channels / sebas-acp / dirs 及它们的中立传递依赖。"
    );
}

#[test]
fn sebas_node_still_depends_on_no_control_plane_role() {
    let names = dep_names("sebas-node");
    let offenders: Vec<&str> = FORBIDDEN_FOR_NODE
        .iter()
        .copied()
        .filter(|f| names.iter().any(|n| n == *f))
        .collect();
    assert!(
        offenders.is_empty(),
        "sebas-node 的依赖图出现了主控角色实现 {offenders:?}；执行节点产物内\
         不得出现 core / webui / router / im 的实现（execution-node spec\
         「Execution node process persona」）。"
    );
}

/// spec「the shared layer is reachable from every role」：每个角色 crate
/// 都经**普通 path 依赖**（直接依赖）够得着 sebas-domain——没有任何 crate
/// 因为够不着原定义而被迫复刻一份（依赖图机械可核对，与 4.2 的 grep 清单
/// 互为印证）。
#[test]
fn shared_layer_is_reachable_from_every_role() {
    const ROLES: &[&str] = &[
        "sebas",
        "sebas-webui",
        "sebas-dispatch",
        "sebas-router",
        "sebas-im",
        "sebas-node",
    ];
    for pkg in ROLES {
        let names = dep_names(pkg);
        assert!(
            names.iter().any(|n| n == "sebas-domain"),
            "{pkg} 的直接依赖里没有 sebas-domain；共享域概念必须经共享层取用\
             （add-domain-layer spec「Neutral leaf dependency」/「reachable \
             from every role」），不允许另立副本。"
        );
    }
}

/// `sebas-node-link` 是中立链路契约 crate，两个纪律都必须放行——钉住这个
/// 「按完整包名匹配」的行为，防止日后有人改成子串匹配误伤它。
#[test]
fn node_link_is_neutral_and_allowed_everywhere() {
    assert!(
        dep_names("sebas-domain")
            .iter()
            .all(|n| n != "sebas-node-link"),
        "域层今天并不依赖 node-link；若未来出现依赖它也是合法的（本测试只\
         防角色实现）"
    );
    // node 自身的树里有 node-link（直接依赖）。
    assert!(
        dep_names("sebas-node")
            .iter()
            .any(|n| n == "sebas-node-link"),
        "sebas-node 依赖 sebas-node-link 属预期"
    );
}
