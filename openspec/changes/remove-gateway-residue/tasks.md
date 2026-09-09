## 1. 删除死脚本

- [x] 1.1 删除 scripts/e2e_gateway_admin.sh，确认无仓库内引用（tasks.py/invoke/docs/CI）
- [x] 1.2 删除 scripts/rewrite.sh，确认无仓库内引用
- [x] 1.3 复核删除后 scripts/ 不再 spawn `sebas gateway` / 写 `[gateway]` 节（grep 全仓确认剩余 0 处，拟真词除外）

## 2. 修正活脚本与 CI

- [x] 2.1 scripts/refresh_api_specs.sh：`sebas-gateway/tests/specs` 路径改 `sebas-router/tests/specs`，注释同步；运行一遍确认克隆目标目录存在
- [x] 2.2 scripts/check_coverage.sh：检查残留 `gateway` 引用（注释已描述改名史），确认阈值随 sebas-router 目录、无死路径
- [x] 2.3 scripts/e2e_router.sh / test_watchdog_debug_upgrade.sh：清理 `GATEWAY_*` 变量名与 `GATEWAY DOWN` 等口语输出为 `ROUTER_*` / router，行为不变；dry-run 或 grep 复核
- [x] 2.4 .github/workflows/ci.yml：注释 `sebas-gateway` → `sebas-router`（指代 process_e2e 所在 crate）

## 3. 修正文档与注释

- [x] 3.1 docs/acp-opencode-accept-2026-09-04.md、docs/acp-opencode-smoke.md：`SEBAS_GATEWAY_PROVIDER_OVERLAY` 旧 env 引用改 `SEBAS_ROUTER_PROVIDER_OVERLAY`（保留历史 log 文件的其余内容不动）
- [x] 3.2 docs/architecture/process-ipc-subcommands.md、docs/frontend-dev.md：现行姿态的 `gateway` 措辞改 router（退役路径 `/gateway`、历史说明不动）
- [x] 3.3 docs/design-history.md / docs/superpowers/specs/2026-08-29-*.md：仅改以"现行事实"出现的 gateway（含 `sebas gateway`/`src/gateway_cmd.rs`/`gateway` 双协议段）；明确历史/原文引用保留
- [x] 3.4 README.md：确认目录树 `原 sebas-gateway` 历史说明保留、其余现行提法已 router
- [x] 3.5 config/config.toml.example：确认注释无旧命令面引用（`my-gateway` 示例保留）
- [x] 3.6 注释修正：src/lib.rs（"hidden alias `gateway`"→无别名）、sebas-webui 前端 settings-modal/router_client/vite.config 等仅剩注释措辞
- [x] 3.7 全仓最终 grep 复核（排除 archive/**、target）：`gateway` 仅剩 拟真词 / 历史说明 / 退役路径 / 会话别名 四类，无现行命令面引用
- [x] 3.8 openspec/specs/webui/spec.md：实施中发现的现行术语残留（`/api/gateway`、`GatewayInfo`、gateway 段措辞）改为现行 `/api/router`/`RouterInfo`——超出 proposal 原声明的措辞级追加，无行为变化

## 4. 收尾

- [x] 4.1 运行相关质量门禁确认无行为改动：cargo test（router/dispatch/webui 单测相关）、已删除脚本无 invoke 依赖
- [ ] 4.2 提交流程遵循仓库约定（conventional commit 单句），feat 分支
