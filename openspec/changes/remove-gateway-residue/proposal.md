## Why

rename-cli-surface 与 fix-spec-gateway-residue 之后，代码面已全面改用 `router`，但一批文档、脚本、CI 注释仍活在旧命令面里：仍把 `sebas gateway` / `[gateway]` / `SEBAS_GATEWAY_*` / `sebas-gateway` 当现行事实引用——引用的是已删除的命令、配置与 crate，是货真价实的死引用，会误导后续维护与自动化。目标：活跃仓库里不再出现指代"模型路由模块"的 `gateway`，让这个词彻底空出来，供将来其他模块使用。

## What Changes

- **文档**（docs/、README.md、.github/workflows/ci.yml、scripts/ 头部注释、config/config.toml.example）：把引用已删除命令/env/配置节/crate 的 `gateway` 死引用改写为现行 `router` 术语（命令 `sebas router`、配置节 `[router]`/`[watchdog.router]`、env `SEBAS_ROUTER_*`、crate `sebas-router`）。
- **脚本 scripts/**：死脚本 `e2e_gateway_admin.sh`、`rewrite.sh` 删除（没有任何 invoke 任务或文档引用，主体是已删除的 `sebas gateway` 命令，无法简单翻新）；`refresh_api_specs.sh`、`check_coverage.sh`、`e2e_router.sh`、`test_watchdog_debug_upgrade.sh` 内的 crate 路径/术语/环境变量残留同步修正。
- **注释**（src/lib.rs、sebas-webui 前端/router_client 等处）仅剩的 `gateway` 措辞顺手改 `router`。

## Non-goals

- 不动拟真保留词：HTTP 语义 `502 Bad Gateway`（router proxy/admin、5 个 router 测试、router_client、gateway_bff_test 等）；配置示例 `my-gateway.example` 纯拟真；webui 退役 IA-v1 路径 `/gateway` 的重定向/归一化行为（现行设计）；`sebas-gateway` 的历史说明（README 目录树、docs/design-history ADR）。
- 不动 `sebas-dispatch` 的 `/gateway` 会话命令别名：它是 webui/IM 命令词（与 CLI 别名不同层），语义现行且被 `parses_router_actions` 附近注释与既有约定维系，改名需跨前后端协同，超出本 change 范围。
- 不改 openspec/specs 的需求语义（主 spec 已无现行术语残留，只有"pre-rename 值应被拒"的需求语句——保留，那是护栏）。
- 不动 openspec/changes/archive/** 历史快照。
- 不新增/删除行为面、不改运行时逻辑。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `cli-service`: 移除"用户运行 `sebas gateway` 报错"的**需求段**——子命令树层面不再定义、不再测试该拒绝路径，旧命令词不再出现在任何现行规范语句中（历史迁移测试归属降低，避免与"gateway 已成自由词"的现状矛盾）。

## Impact

- 文档：README.md、docs/architecture/process-ipc-subcommands.md、docs/frontend-dev.md、docs/acp-opencode-accept-2026-09-04.md、docs/acp-opencode-smoke.md、docs/superpowers/specs/2026-08-29-agent-core-architecture-design.md（历史设计稿，措辞修正为 router，保留原文为 gateways 的 ADR-3 不动）。
- 脚本：scripts/{e2e_gateway_admin.sh,rewrite.sh} 删除；scripts/{refresh_api_specs.sh,check_coverage.sh,e2e_router.sh,test_watchdog_debug_upgrade.sh} 术语修正。
- CI：.github/workflows/ci.yml 注释修正。
- 注释：src/lib.rs、sebas-webui 前端/router_client.rs 等注释措辞。
- 配置：config/config.toml.example（保留 `my-gateway` 示例，仅改描述性注释）。
- specs：openspec/specs/cli-service/spec.md（删一个需求段）；实施中追加 openspec/specs/webui/spec.md 的现行术语措辞修正（`/api/gateway`→`/api/router` 等，无行为变化）。
- 无运行时代码逻辑改动。
