## Why

运营者实证：会话明明没有任务在执行，所有新输入却全部落进「待执行」栈且永不执行，条目删除与上下移动点了也没反应。设计 review 定位出三个结构性缺陷：① 队列前进 100% 依赖「终态事件到达」单一信号，无超时、无看门狗、无用户可见解释，任何一环丢失（子进程静默死亡、事件流断线、泊车审批无人应答）即永久饿死；② 前端 `turnInFlight` 只认 `status_slug === 'working'`，而 waiting（泊车审批）/starting（spawn 窗口）态下提交按钮呈普通 send 形态，后端却因卡片仍是 WORKING 而静默入队；③ pending 管理操作的拒绝被设计为绝对静默（原 D8「绝不弹错」），确定性拒绝与网络失败一律无感，按钮形同虚设。

## What Changes

- 引擎停滞看门狗：会话卡片处于 WORKING、无泊车审批、且持续 `turn_stall_timeout`（默认 600s，0 = 关闭）未收到该会话任何事件时，强制收尾到 DONE、drain 队列，并发一条 warn 级通知点名会话与搁浅条目数。
- 提交控件的「turn 在飞」判定与后端对齐：waiting（含泊车审批）与 starting 态下输入非空即呈排队形态并附「将排在当前回合后」提示；只要回合实际在飞（含泊车中），停止方块即可达。
- 待执行栈条目注明不前进的原因：「等待你的审批」/「等待当前回合结束」+ 起等时刻。
- 拒绝反馈分级：确定性拒绝（未知 id、越界、越优先、操作后服务端全量里条目仍在）经分级通知的低档就地呈现并点名原因；竞态竞输（服务端真相已按操作意图收敛、条目已不在）保持静默对账。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `core-session-channel`: 新增「待执行队列前进性」要求——队列不得无限期依赖单一终态事件，停滞必须有兜底收尾与可见通知。
- `agent-workbench`: 修订「Submit control reflects submission and turn state」（在飞判定扩展到泊车/启动态、停止可达）与「Pending submissions stack above the composer」（条目注明等待原因、确定性拒绝可见反馈）。

## Impact

- `sebas-dispatch` engine：停滞检测任务、强制收尾路径、泊车审批状态的判读。
- `sebas-webui` frontend：`workbench-composer.ts`（提交控件判定）、`pending-stack.ts`（原因标注 + 拒绝反馈）、`api/client.ts`（拒绝明细透出）。
- 配置：`[dispatch]` 新键 `turn_stall_timeout`。
- 测试：dispatch 单测（停滞收尾、泊车豁免）、前端单测（判定与反馈）、e2e 补「中途杀子进程 → 队列自愈」用例。

## Non-goals

- 不改「忙中提交自动排队」的语义本身（back-pressure 保留）。
- 不做逐回合超时强杀（claude 驱动已有 hang 升级链：interrupt ×3 → SIGTERM）。
- 不新增 HTTP/WS 端点。
- 不动 staging（spawn 窗口）语义与远端节点会话（节点侧自主恢复）。
