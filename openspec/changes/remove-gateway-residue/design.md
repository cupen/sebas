## Context

参见 proposal.md —— rename-cli-surface / fix-spec-gateway-residue 已完成代码与主 spec 的改名，本 change 清掉**活跃仓库**里把旧命令面当现行事实的 `gateway` 死引用。现状勘察结论（来源：全仓逐行 grep，排除 archive/）：

- 运行时代码已无 `sebas gateway` / `[gateway]` / `SEBAS_GATEWAY_*` 支持；`src/cli.rs` 的 `Cmd` 枚举既无 `gateway` 也无 `watchdog` 变体（clap 对两者都是 unknown subcommand）。
- 残留集中在四类：文档/CI 注释（docs/、README、.github、config example）、脚本（scripts/）、`sebas-dispatch` 的 `/gateway` 命令别名、spec 里"拒绝旧命令"的护栏场景。
- 拟真/历史保留词必须避开：HTTP `502 Bad Gateway`（约 14 处）、`my-gateway.example` 配置示例、webui 退役路径 `/gateway` 重定向、`sebas-gateway` 历史说明、`openspec/changes/archive/**`。

## Goals / Non-Goals

**Goals:**
- 活跃仓库内 `gateway` 只保留"拟真词 / 历史记录 / 退役路径"三类含义，不再有指代现行"模型路由模块或命令面"的用法。
- 明确"gateway 是自由词"：本 change 之后，新模块使用 gateway 命名不再与旧模块残留冲突。

**Non-Goals:**
- 不做任何运行时行为/CLI 语义改动（代码只有注释变化）。
- 不为"词法洁癖"改拟真词与历史快照（见 proposal Non-goals）。

## Decisions

### D1: dispatch `/gateway` 别名保留，但撤掉 CLI spec 中"拒绝 gateway"的护栏场景
`sebas-dispatch` 的 `/gateway` 是 **webui/IM 会话内的命令词**（与 `src/cli.rs` 的 CLI 子命令别名不同层），语义现行、无运行时替换需求；改名需跨前端与既有按钮联动，超出本 change。而 CLI spec 的 `pre-rename subcommand aliases rejected` 场景守护的 `sebas gateway` 在代码里早已不存在（`Cmd` 无此变体，clap 天然 unknown-subcommand）——护栏守护的对象已消失，继续把 `gateway` 写进 spec 反而与"gateway 已成自由词"矛盾。因此：移除该场景，`watchdog` 一并移除（同为无变体旧词，clap 行为一致），以泛化场景 `unknown subcommand rejected` 替代（守护真实仍存在的行为——未知子命令报错退出）。

*替代方案*：保留护栏场景仅删 `gateway`。否决——它测的是 clap 默认行为而非任何定制约束，spec 不该为框架默认行为背书；且保留 `sebas gateway` 字样与目标相悖。

### D2: "gateway 残骸"判定标准（识别什么改、什么留）
逐个出现点按下表归类，只有第一类动：

| 类别 | 含义 | 处置 |
|---|---|---|
| 死引用 | 引用已删除的 `sebas gateway` 命令、`[gateway]`/旧 `[router]` 配置节、`SEBAS_GATEWAY_*`/`SEBAS_AGENT_GATEWAY_*` env、`sebas-gateway` crate、`gateway` 子命令/flag | 改写为现行 `router` 表述 |
| 拟真词 | `502 Bad Gateway`、`my-gateway.example`、第三方错误类型名 | 不动 |
| 退役路径 | webui `/gateway` → `/` 重定向（IA-v1 退役设计） | 不动 |
| 历史记录 | `sebas-gateway` 历史说明、ADR、archive/** | 不动 |

## Risks / Trade-offs

- [删除脚本可能仍被仓库外流程或历史分支引用] → 删除前在 docs/openspec/tests 全量复核引用；`e2e_gateway_admin.sh`/`rewrite.sh` 的验收职责已由 `testsuite-e2e`/`testsuite-acceptance` 与 `e2e_router.sh` 承接，CI 无引用。
- [误改拟真词/历史句（如 docs/design-history ADR-3、superpowers 设计稿的原文引用）] → 严格按 D2 判定表执行；对历史设计稿只改"以现行姿态出现"的句子，保留原文标记。
- [spec delta 复制整段 Requirement 时丢失内容] → 逐字对照现 spec 全文复制，仅删/改目标场景。
