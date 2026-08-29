## Why

`docs/superpowers/` 是采纳 OpenSpec 前的历史工作区(11 份设计文档 + 12 份实施计划,约 17.5k 行)。`bootstrap-specs`(已归档)已把行为语义回填进 `openspec/specs/`,这些文档与现 specs 大量重复或已被取代,继续留存会误导检索。而代码与文档里还有约 209 处指向它们的引用——删除后全部成为死链,须先同步、再清理、最后删除。

## What Changes

**先同步**:
- 新建 `docs/design-history.md`(ADR 式,每条:背景/决策/后果):弃 ACP 直连 SDK、卡片流模型选型、gateway 协议面与 per-key 简化、provider state v2 统一、provider 评审决策记录摘要。
- `docs/review/2026-08-17-code-design-audit.md` 的 5 条建议(P1×1/P2×2/P3×2)逐项核对现状:已修→记入 ADR;未修→开 beads issue(P1「webui 非 loopback 应在 watchdog 层拦截」优先)。
- provider 评审遗留承诺「gateway TOML routes 后续由 webui 编辑」开 bead。

**再清理引用**(全量,已确认):
- 12 处显式 `docs/superpowers/` 路径:README×2、config.toml.example×2、src/cli.rs、gateway_cmd.rs、gateway/lib.rs、acp-claude×3、router/maps.rs、docs/perm-flow/sequence.md → 改指 openspec specs 或内联。
- 64 处带日期 `spec 2026-08-17 §N`(15 个 .rs + .claude/rules/how-to.md)。
- 133 处裸 `spec §N`(46 个 .rs,按约 5 个家族映射:卡片流→`feishu-cards`、gateway→`gateway-*`、provider→`provider-management`、命令→`cli-service`/`watchdog`、acp→`acp-driver`);无对应 spec 的就地内联事实或删引用标签。

**最后删除**:
- 删除 `docs/superpowers/**`(23 文件)与 `docs/review/2026-08-17-code-design-audit.md`(`docs/review/` 随之清空移除)。
- `xtask` 新增 `check-docs` 子命令:扫描 `docs/superpowers` 路径与 `spec 2026-`/裸 `spec §` 引用模式,防回归。

## Capabilities

### New Capabilities

(无 — 纯文档与开发工具变更,运行时行为不变,已设 `skip_specs: true`)

### Modified Capabilities

(无)

## Non-goals

- 不改任何运行时行为;行为语义已在 specs 中,不再重复迁移。
- 不处理 `openspec/changes/archive/` 内的历史提及(归档文档允许引用已删除文档)。
- 不修复审计文档指出的代码问题本身——只负责处置与开 bead。
- 不引入构建步骤、npm 或新外部依赖。

## Impact

- 删除:`docs/superpowers/`(23 文件)、`docs/review/`(1 文件)。
- 新增:`docs/design-history.md`、`xtask` 的 `check-docs`。
- 修改:约 60 个文件中的注释/README/config 指针。
- 无 API、协议、配置格式变化;无新增 Rust 依赖。
