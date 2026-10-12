# add-pi-install-recipe — Tasks

## 1. 配方表与前置（sebas-webui/src/agent_install.rs）

- [x] 1.1 `Recipe` 增可选前置字段（`prereq: Option<Prerequisite>`，`Prerequisite::NodeAtLeast { major: u32, minor: u32 }` 一种形态），`RECIPES` 的 claude/opencode 两行填 `None`。验证：单测断言两既有配方 `prereq.is_none()`。
- [x] 1.2 `RECIPES` 追加 pi 行（`name:"pi"`、`package:"@earendil-works/pi-coding-agent"`、`bin:"pi"`、`driver:"pi"`、`args:&[]`、`prereq: Some(NodeAtLeast{22,19})`）。验证：单测断言 `recipe("pi")` 存在且字段逐项正确、`recipe_agent_definition` 产出 `{driver:"pi", path:<私有 bin 绝对路径>}`（无 args 键）。
- [x] 1.3 Node 前置解析与判定：`node --version` 探测（带超时，复用 `resolve_program`）、版本解析（容忍前导 `v`，解析失败按不满足）、与下限比较；产出 typed 失败（node 缺失 / 版本不足，各带最小指引文案）。验证：单测用 PATH 垫 fake node 脚本（缺失 / `v22.18.9` / `v22.19.0` / `v23.0.0` / 异常输出五种剧本）断言判定与文案。
- [x] 1.4 过期注释修正：`Recipe` 的 driver 注释（现写「claude | acp」）改为三 driver；`RECIPES.len() == 2` 的单测断言改为按名查存在性（不绑长度）。验证：`cargo test -p sebas-webui` 绿。

## 2. 路由与探测回退

- [x] 2.1 handler 插入前置检查步骤：在 npm 在场探测之后、同步安装之前，按配方 `prereq` 解析（`None` 完全跳过），不满足即 typed 4xx 且不发起安装、不建行。验证：集成测试断言 pi 配方在 node 缺失/版本低时 4xx 且 fake npm 未被调用（argv 日志为空）、claude 配方在同样 node 条件下照常安装。
- [x] 2.2 未知 recipe 文案改为不含枚举的耐久措辞（避免每加配方改文案）。验证：集成测试断言未知 recipe 仍 4xx 且文案不再硬编码「claude / opencode」。
- [x] 2.3 `source_from_store_item`（sebas-webui/src/agent_kinds.rs）path 缺省回退按 driver 取默认二进制名（claude → `"claude"`、pi → `"pi"`、acp → 空串维持现状），常量复用 sebas-models 的 `DEFAULT_CLAUDE_PATH`/`DEFAULT_PI_PATH`。验证：单测覆盖 driver=pi 无 path 时 command 为 `["pi"]`、acp 无 path 仍为空 command、claude 行为不变。

## 3. 前端（sebas-webui/frontend）

- [x] 3.1 `settings-modal.ts` 的 `INSTALL_RECIPES` 追加 pi 行（name + package；状态徽标与安装按钮沿用既有机制，无新交互）。验证：Vitest 组件测试断言 pi 配方行渲染、未装显示安装按钮、已装不显示。
- [x] 3.2 前置失败的通知文案经既有 `errorText`/`agentAction` 通道如实呈现（无需新 UI）。验证：Vitest 断言失败通知含后端返回的 node 指引文案。

## 4. 进程级与浏览器验收

- [ ] 4.1 进程级 e2e：扩 `tests/testsuite_e2e_test.rs` 的 `agent_auto_install` 模块——沙箱 + PATH 垫 fake npm/fake node，旅程 = 安装 pi → 响应 driver 正确、目录出现 pi 行、argv 证据；再加一条 node 版本不足 → typed 4xx 且无落盘。验证：`invoke testsuite-e2e --case <name>` 通过。
- [ ] 4.2 浏览器 GUI 验收：`invoke testsuite-webui-sandbox`（auth 关）确认可安装区出现 pi 配方行、安装按钮与通知形态正确；确认真实 npm 未被触发（fake npm 日志为证）。验收清单 + 截图落 `verification/`。

## 5. 回归与收口

- [x] 5.1 全量回归：`rtk cargo test`、`cd sebas-webui/frontend && rtk pnpm test`、`cargo clippy` 与 `cargo fmt --check` 无新告警。验证：命令退出码 0。
- [x] 5.2 规范与工件收口：`openspec validate add-pi-install-recipe --strict` 通过；specs 增量逐场景对账（尤其「node 缺失/版本不足拒绝」「无前置配方不受影响」「pi 安装后目录行以 pi 驱动就绪」三场景）。验证：validate 输出绿。
