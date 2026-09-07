# design — expand-testsuite-webui-coverage

## Context

既有套件已钉死首批旅程：沙箱形态为 `sebas core --router --debug --webui` 单进程 + fake-claude 桩，页面对象与 api helper 齐备，断言纪律为 web-first + 轮询。既有现实约束沿用（见前序 change 的实现期发现：AllowSession 不持久、wa-tree 崩溃、refuse 非终态错误呈现、组合后端 provider 状态）。本期只定覆盖方向与可测性前置，不锁具体 case。

## Goals / Non-Goals

Goals：四面闭环、失败留现场、`--case` 可分流、3 次全绿。
Non-goals：见 proposal（两进程形态、CI、多浏览器、真实 probe、已知 bug 修复）。

## Decisions

### D1. 模型可测性：先让沙箱里出现有模型选项的会话

现状缺口：默认桩会话无模型选项，模型正向语义无从断言。方向（实现期定）：优先扩展测试桩使其透出固定的模型选项与切换事件，改动以测试桩为界、不碰生产驱动；若不可行则降级为只覆盖呈现与拒绝语义。用例断言的是"切换语义"（请求→回流→详情刷新→转写不丢），不是模型智能。

### D2. 复用既有页面对象，按缺口增补不另起炉灶

新增覆盖所需的页面交互 SHALL 复用既有页面对象与 helper，不足处增补 reading 能力（如项目排序与分支呈现、模型下拉与 settings provider 列表、composer 输入守卫与 transcript 顺序断言）。增补只加读测能力，不改既有用例定位；key 编码、深链构造等既有约定沿用。

### D3. 数据准备：沙箱内自给自足

各面所需数据 SHALL 在一次性沙箱目录内自建自销：项目面含 git 与非 git 目录形态；会话面按需建多个会话；对话面覆盖多轮与特殊输入。分支名、顺序等易变值 SHALL 以包含或相对断言为准，不写死字面量。

### D4. 稳定性：沿用既有纪律

新用例沿用既有断言纪律（web-first + 轮询，禁固定 sleep，失败留现场）。管理动作后先等服务端态再断 UI；多轮对话逐轮等收敛再发下一轮；新增用例独立文件、失败定位到文件级。

### D5. 账本与入口：只加行不改矩阵语义

账本在既有能力行下追加覆盖证据，不新开能力行。入口语义不变，既有过滤机制自然容纳新用例。

## Risks / Trade-offs

- [桩扩展污染既有缺省断言] → 新旧会话分流，既有用例不动。
- [交互类断言 flake] → 持久化语义与 UI 交互拆用例，失败不混源。
- [provider 列表依赖 router 状态] → 只断呈现与数量一致，不做真实拨测、不碰真实凭据。
- [用例时长增长] → 轮数与数据量封顶，超时预算按单轮配。

### 探索期已探明约束（2026-09-07，对着代码原文确认，执行时对照）

- [重复注册语义] → `POST /api/projects` 对已 canonical 路径返回 **409**（`api.rs:640`、`projects.rs:127-128`），非幂等 200；用例 SHALL 断 409 + body 包含"已注册" + rail 单行。若将来改为幂等 200，属产品语义变更。
- [空输入守卫形态] → composer 空守卫是 `trim()` 后静默 `return`（`workbench-composer.ts:376-386`、`session-detail.ts:358`），按钮 `disabled` 只看 `sending/unreachable`；用例 SHALL 断"点击/回车后无新回合、无状态变化"，SHALL NOT 断 `button[disabled]`。
- [分支缓存与命名] → `read_branch` 有 30s TTL 缓存（`projects.rs:219`），切分支后 SHALL 以 `reload`（而非轮询）取新值；分支名 SHALL 以 `contains` 断自建名（如 `feat-d3-<uniq>`），SHALL NOT 断 `main`/`master` 字面；非 git 目录 `.branch` 不渲染且不报错。
- [D1 结论：降级 C] → 桩只改自己带不动 Claude 驱动（`SpawnOutcome.model` 恒 `None`，`SetModel` 在碰子进程前短路终态报错，`driver.rs:422-431`）；3.2 按"呈现 + 拒绝语义"执行（非法模型 200 接受 + 快照不变 + transcript 不丢 + provider 行数一致 + 零 probe），3.1 另立项（桩 + 驱动最小模型面 + 双 agent 沙箱）。
- [restore 不复活会话] → `archive_session` 先 close（mapping + transcript 丢弃，`engine/mod.rs:1282-1289`），`restore_session` 只删归档条目（`api.rs:864-868`）；恢复后详情页如实 404（同 crash 旅程），与 project-session-actions「History 点击恢复可写」spec 条文不一致。用例 SHALL 按诚实语义断（History 条目消失 + 404 + archive 列表干净），SHALL NOT 断"恢复后可继续发消息"；产品语义变更（归档免 close / 恢复重建映射）另立项，本 change 不修。
- [无模型会话 set_model 是终态杀伤] → webui 只投递（200 ok，`session_backend.rs:448-467`），Claude 驱动以终态 Error 应答 SetModel（`driver.rs:422-431,533-544`），会话被拆除（同 crash 语义）。用例 SHALL 按终态诚实断（行离开列表 + 404 + 详情页如实呈现死亡），SHALL NOT 断"模型不变会话存活"；驱动支持模型面属 D1 重立项范围，本 change 不修。
- [reorder 必须发全量] → 部分路径 reorder 把尾部交给 added_at 顺序，而同秒注册的 added_at 打平后服务端 HashMap 尾序随机（实测 7 跑 flake 2 次）；用例 SHALL 每次发全量路径钉死顺序，SHALL NOT 依赖尾部追加顺序。

## Migration Plan

纯增量：新用例 + 页面对象增补 + 桩扩展（可选）+ 账本行。回滚 = 删新增物，无生产改动。

## Open Questions

- D1 已定：降级 C（见 Risks「D1 结论」；2026-09-07 探索确认，桩-only 方案 A 不可行）。
