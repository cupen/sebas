## Why

第四轮 WebUI 全链路黑盒 GUI 验收（Playwright 驱动真实 Chromium + fake-claude/fake-acp 沙箱，覆盖登录
RBAC、项目注册、四 agent 会话、thinking/tool use、权限卡 allow/deny、四模式切换、slash 命令面板、
流式、模型选择与类型化拒绝、归档恢复、会话表与深链、Settings 六分区、用户管理、移动端断点）发现
2 个确认缺陷与 3 个打磨项。其中 P0 级缺陷是：claude 驱动的 5 分钟静默升级杀掉子进程后，整个会话
从状态中消失（列表移除 + 详情 404），操作员丢失全部对话历史且无任何告知；P1 级缺陷是：回合处于
「已收到（agent 尚无首帧输出）」阶段时 composer 不提供停止控件，操作员面对卡死回合在 600s 看门狗
兜底前无任何自助手段。

## What Changes

- **「已收到」阶段可取消（P1，实现修复 + spec 澄清）**——回合被服务端接受、但 agent 首个输出条目
  尚未落地的接收回执阶段，composer 的停止控件必须可达（空输入时呈停止方块），激活即按既有
  interrupt 语义取消该回合。现实现把 `turnInFlight` 派生自 working/engaged 态，接收回执阶段被漏掉。
- **升级击杀保留会话（P0，实现修复 + spec 澄清）**——claude 驱动静默升级（interrupt 阶梯）终止
  子进程后：受影响回合以可见的错误条目收尾、会话记录与转录保留（列表/详情/回放可及）、排队提交
  按既有 pending-queue 语义如实释放上报、下一条消息照常孵化全新会话。现实现把会话整个从状态中
  移除（列表消失、详情 404），违反「terminal teardown 只清活跃绑定、不抹历史」的既有语义。
- **归档→恢复保留会话命名来源（P3）**——恢复重建会话时保留归档前的首条消息预览与 label；
  现实现恢复后 `prompt_preview` 变空串，rail 名退化为原始 session 引用（`web-1789…`）。
- **P3 瑕疵批量打磨（无 spec 变化）**：rail History 条目长路径撑出横向滚动条（截断省略）；
  ≤640px 移动端会话头部权限模式章逐字竖排换行（不允许逐字断行）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`：「Submit control reflects submission and turn state」——in-flight 定义扩展到
  接收回执阶段（提交已接受、agent 首个输出条目未落地），该阶段空输入必须呈现停止控件。
- `session-lifecycle`：「Terminal error teardown」——澄清 teardown 清的是活跃绑定与映射，会话记录
  与转录必须保留可回放；补升级击杀场景。
- `acp-driver`：「Hang detection with escalating kill」——击杀阶梯走完后受影响回合以可见错误收尾、
  会话不得从操作员可见状态中消失。
- `project-session-actions`：「Session rows are named by the first prompt」——恢复后的会话行命名
  来源（label/首条消息预览）跨归档保留，不得退化为短 id。

## Impact

- `sebas-webui/frontend/src/views/workbench-composer.ts`（submitState 的 in-flight 判定扩展到
  接收回执阶段；停止控件复用既有 interrupt 调用）
- `sebas-acp/src/claude/driver.rs` 或会话引擎层（升级击杀后的回合收尾与保留语义——具体删除点在
  实现期定位，tasks 首项即定位步骤）
- `sebas-webui` 会话列表/详情读路径（若有参与移除，需同步改为保留）
- 归档恢复路径（`sebas-webui/src/archive.rs` 或 dispatch 重建逻辑）的 prompt_preview/label 迁移
- `sebas-webui/frontend/src/views/project-rail.ts`（History 长路径截断）与
  `app-shell.ts` / 会话头部样式（移动端模式章换行）
- 不改变 wire 协议既有字段；interrupt/pending-queue 语义沿用既有 spec。
