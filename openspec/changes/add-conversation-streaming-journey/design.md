## Context

见 `proposal.md`。设计相关的现状约束：

- 现有唯一流式用例 `tests/testsuite-webui/tests/streaming.spec.ts` 的 oracle 是**服务端 API**（`getSession` 轮询 chunk 条目）+ 回合结束后的 DOM 计数，`working` 瞬态只进 annotation。
- 传输侧已有 `turn.append` 帧与前端订阅（`sebas-webui/frontend/src/views/transcript-view.ts:1098` 起按 `session_id` 过滤、按 `position` 去重并入渲染管线）——**管线存在，缺的是能证伪"只在结束时上屏"的断言**。
- 桩能力约束：`tests/bin/fake-claude.rs` 的 `--slow-ms` 是 `settle_pause()`（收尾前一次性停顿，`fake-claude.rs:844`），默认场景的文本 delta **背靠背**发出。
- 真正的时限是 driver 看门狗每秒发出的**控制探针应答超时 1.5s**（`sebas-acp/src/claude/driver.rs:477`）——探针悬空即判子进程死亡；而**挂起探测**是另一回事，默认 **5 分钟**（`driver.rs:498-502`，可用 `SEBAS_HANG_TIMEOUT_SECS` 覆盖）。
- 本会话实测：`--slow-ms 5000` 的沙箱会话在约 2.5s 由 `working` 直接转 `dormant`——`settle_pause` 是同步 sleep、期间不应答探针，1.5s 应答超时先到。故该窗口不能靠拉长 `--slow-ms` 构造。

## Goals / Non-Goals

**Goals:**

- 让「回合进行中前端已增量上屏」成为**可在浏览器 DOM 上确定性断言**的事实。
- 用例在**首次尝试**内通过，不以 retry 兜底；零固定 sleep。
- 若断言证伪实时性，则修到绿（用例即交付物）。

**Non-Goals:**

- 不覆盖"回合起不来静默排队"（另立 change）。
- 不重写既有 API 级流式断言（保留其"一回合=一气泡"聚合口径）。
- 不引入真实模型凭据，不改 driver 的 watchdog 预算数值。

## Decisions

**D1：oracle 用「回合进行中的 DOM 文本」，不用 API，也不采用 annotation。**
理由：只有 DOM 中途断言能区分"实时渲染"与"结束整块渲染"；API oracle 对二者同在。备选（保留 annotation）已证明会放过缺陷，弃用。

**D2：确定性来自桩的"间隔发 delta"能力，而非拉长收尾停顿。**
给 `fake-claude` 增加 `--delta-gap-ms N`（默认 0 = 现行行为）：默认场景的每段文本 delta 之间 sleep N 毫秒，且 gap 内每 50ms 应答控制探针（该路径因此不受 1.5s 探针应答超时约束）；`drip` 触发词的段间距同步采用 N，而该路径先 sleep 再 pump，故其 gap 必须 < 1.5s。取 N=500、3 段 delta →「回合进行中」窗口约 1s。备选：把 `drip` 也改为 gap 内持续应答探针以支持更长窗口——改动更大，留待确有更长窗口需求时再做。

**D3：新增**专用 **spec `conversation-streaming.spec.ts`，不改名复用旧文件。**
旧 `streaming.spec.ts` 保留其"多 chunk 合并为一气泡"的聚合断言（仍有价值），但把 `working` 的中途断言从 annotation 升级为新用例的硬断言；两份用例职责分离：聚合 vs 实时。

**D4：用例以"聚焦 placeholder + composer 提交"起步，避免快照路径让断言落空。**
若先建带 prompt 的会话再打开页面，回合可能在页面加载前完成，中途断言会退化为空断言。做法：建 placeholder 会话 → 打开其深链 → 在 composer 输入并提交 → 以 50ms 粒度轮询 DOM 与 `status_slug`，断言"文本已出现"发生在"终态"之前。

**D5：该 spec 局部 `retries: 0`。**
套件全局 `retries: 1` 会把实现缺陷掩盖成 flaky。本用例是"拦得住"的门面，要求一次通过；其他 spec 的 retry 策略不动。

**D6：若用例红，修前端渲染或 WS 中继到绿。**
优先在 `transcript-view.ts` 的 `turn.append` 应用路径上查（订阅、position 去重、`willUpdate` 裁剪），其次查 webui 侧 `turn.append` 的合并/推送时机。

## Risks / Trade-offs

- [watchdog 预算把 gap 卡死] → gap 默认 500ms 且可配；帧间静默必须 < 1.5s。需要更长窗口时再实现"gap 期间应答探针"。
- [采样窗口太短导致假红] → 至少 3 段 delta + 50ms 采样 + 在"未终态"条件下等待文本出现；不使用固定 sleep。
- [局部去 retry 暴露其他不稳定] → 仅本 spec 生效，问题会被显式暴露而非掩盖，符合本 change 意图。
- [快照与帧竞态使断言偶发] → 采用 D4 的 placeholder 起步，确保观测起点在回合开始之前。
- [已知残余缺口：dashboard 首帧守卫] → `sebas-webui/frontend/src/dashboard.ts` 的 `applyTurnAppend` 以 `cached.length === 0` 早退，把「首拉未落地」与「首拉落地为 0 条」混为一谈，导致 0-turn 占位会话首回合的最早 `turn.append` 帧被丢；实时链实际依赖随后的 HTTP 快照（实测 +22ms 内完成，故本用例稳定绿）。若首个快照迟于整回合，实时性会退化为「结束整块」，与 `agent-workbench` 增补段的字面不符。最小修法（区分 `undefined` 与空数组、空缓存游标取 -1）已建 beads `sebas-8517` 跟踪，属 follow-up，非本 change 阻塞项。
- [thinking delta 无实时覆盖] → `agent-workbench` 增补段提到 thinking delta，但桩的 `thinking` 场景不消费 `--delta-gap-ms`，当前无法构造「思考增量进行中窗口」；本 change 只覆盖文本 delta 的实时上屏。
- [同名 scenario 歧义] → 主 spec 在 `会话核心旅程` 与 `agent 对话覆盖` 下各有一条「流式分批渲染」，账本只锚后者；消歧留作后续。

## Migration Plan

无数据/部署迁移。回滚即删除新 spec、还原 `fake-claude` 的新增 flag 默认值（0 = 现行行为），并还原两条 spec 需求文本。

## Open Questions

无（gap 数值与用例名属实现细节，可在 apply 阶段按实测微调，不影响 spec 与方案）。
