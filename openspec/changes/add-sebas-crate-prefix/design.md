# 设计：workspace 成员 crate 统一 sebas- 前缀

## Context

workspace 共 6 个成员：根包 `sebas` + 5 个待改名 crate（`router`、`feishu`、`acp-claude`、`gateway`、`webui`）+ `xtask`。crate 名出现在约 286 处 `use` 导入、4 份 Cargo.toml 的路径依赖、Dockerfile COPY、xtask 内硬编码路径与文档中。所有 crate 均未发布 crates.io，无外部消费者；CI 与脚本只使用 workspace 级命令（`cargo build --bin sebas`、`cargo test --workspace`），不引用成员包名。动机见 proposal.md。

## Goals / Non-Goals

**Goals:**
- 5 个成员 crate 包名与目录名统一改为 `sebas-` 前缀，目录名与包名一致
- 全仓编译、测试、Docker 构建在改名后保持通过
- 文档与规划工件中的 crate 引用同步更新

**Non-Goals:**
- 不改运行时对外名称：CLI 子命令、`config.toml` 配置段、`state/` 文件名、openspec capability 名（如 `specs/webui/`）
- 不改 `xtask`、`fake-claude`、根包 `sebas` 的命名
- 不改任何运行时逻辑（除导入路径、清单、目录名、xtask 路径常量）

## Decisions

**D1 目录与包名同时改，保持二者一致。**
备选是只改 package 名、目录不动——改动面更小，但目录名与包名长期分叉会持续制造困惑。目录用 `git mv` 保留历史。

**D2 导入路径用 Rust 默认规则（连字符→下划线），不设 `lib.name` 绕行。**
即 `use sebas_router::`。备选是在每个 crate 里设 `lib.name = "router"` 保持旧导入——被否决：这会让对外包名与代码内名不一致，正好违背本次"命名可辨识"的目标。

**D3 一次性原子重命名，单个 feat 分支完成。**
备选是逐 crate 渐进改名——中间态命名混杂，且各 crate 互相依赖（router→feishu/acp-claude，webui→三 者），渐进无收益。改名映射：

| 现名 | 新包名 | 新目录 | 代码内导入 |
|---|---|---|---|
| router | sebas-router | sebas-router/ | sebas_router |
| feishu | sebas-feishu | sebas-feishu/ | sebas_feishu |
| acp-claude | sebas-acp-claude | sebas-acp-claude/ | sebas_acp_claude |
| gateway | sebas-gateway | sebas-gateway/ | sebas_gateway |
| webui | sebas-webui | sebas-webui/ | sebas_webui |

**D4 xtask 不加前缀，但其硬编码路径必须改。**
xtask 的 models 同步功能按 `<repo>/gateway/src/models.rs` 相对路径定位目标文件（`xtask/src/main.rs` 多处），需同步改为 `sebas-gateway/src/models.rs`。这是 xtask 唯一受影响点；`check_docs.rs` 引用的 openspec capability 路径不变。

**D5 活跃变更工件只改"明确指 crate"的引用。**
`add-core-session-channel`、`add-project-workbench` 等工件中的 `webui`/`router` 字样，一部分指 capability（`specs/webui/`，不改），一部分指 crate（如 `webui crate`、代码引用，需改）。逐一判断，避免误改 capability 名。

**D6 不设 spec delta（skip_specs: true）。**
二进制名、CLI、配置格式、API 均不变，无 spec 级行为变化，不为满足校验而编造能力需求。

## Risks / Trade-offs

- [约 286 处导入改动引入编译错误] → `cargo check` + `cargo test --workspace` 全量门禁；CI 命令不变，天然验证
- [改名与 `redesign-webui-console` 剩余任务（5.1 模板重样式）并行冲突] → 5.1 只动 webui crate 内模板/静态资源，不涉导入；先落地本变更再继续即可
- [隐藏的路径/文档引用遗漏] → 以 crate 名与下划线形式做全仓 grep 扫描兜底；Dockerfile 单独 docker build 验证
- [未来发布 crates.io 时名称被占用] → 当前未发布无影响；若未来发布，届时再检查名称可用性

## Migration Plan

1. `feat/*` 分支上：Cargo.toml 清单 → `git mv` 目录 → 全量替换导入 → xtask 路径常量 → Dockerfile → `cargo test --workspace` + docker build 验证
2. 文档与活跃变更工件更新
3. 合并回 main（先 rebase，`--no-ff`），回滚策略为 revert 该 merge commit

## Open Questions

- 无。未来是否发布 crates.io 不影响本次决策（见 Risks）。
