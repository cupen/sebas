# im-service Specification

## Purpose
独立 IM 服务（`sebas-im` crate + `sebas im` 进程）：作为 webui 的兄弟服务经核心会话通道观察并驱动会话，宿主全部 IM 适配器（飞书第一个）与 IM 交互状态机（卡片、审批卡、命令、表单、reactions），使 core 成为不含任何 IM 的纯会话核心。

## Requirements

### Requirement: im 作为独立服务进程

sebas SHALL 提供 `sebas im -c <config>` 子命令启动独立 IM 服务进程，与 `sebas webui` 同构。im 进程 SHALL 经核心会话通道（Unix socket + 共享密钥）访问核心，SHALL NOT 在本进程内持有 `RouterHandle`、会话映射或 spawn 任何 agent 子进程。watchdog SHALL 将 im 作为受管子进程托管（spawn、崩溃退避重启、状态如实上报、`ServiceSet`/`ServiceRestart` 可操作），配置节为 `[watchdog.im]`。

#### Scenario: im 独立进程经通道驱动会话

- **WHEN** im 进程收到一条飞书文本并把消息发给核心
- **THEN** 会话的创建/复活/回复全部由核心经通道完成，im 只从通道响应与事件流得知结果

#### Scenario: im 崩溃被 watchdog 重启

- **WHEN** im 子进程意外退出
- **THEN** watchdog 按崩溃退避策略重启它，`ServiceStatus` 反映重启中的真实状态

### Requirement: 多 IM 适配器宿主

im 服务 SHALL 维护自己的适配器注册表（复用 `channels` 中立抽象）：按配置注册已启用的 IM 适配器，飞书为第一个入驻者；注册新 IM 适配器 SHALL NOT 需要修改核心。core 进程 SHALL NOT 注册任何 IM 适配器，且 SHALL NOT 依赖 `sebas-feishu` 或 `sebas-im`。

#### Scenario: feishu 注册进 im 的注册表

- **WHEN** 配置启用飞书且 im 服务启动
- **THEN** `feishu` 出现在 im 进程的活跃通道注册表中，core 进程的日志与依赖里没有 feishu

#### Scenario: 停用的通道不在注册表

- **WHEN** 某 IM 通道未启用
- **THEN** im 不注册该适配器、不建立其传输连接，该通道无任何出入站活动

### Requirement: IM 交互状态机随迁

卡片状态机（每会话单卡片、流式合并、预算与轮换——中立契约遵循 `channels`「Neutral presentation content contract」，飞书渲染遵循 `feishu-cards`）、权限审批卡、acknowledgment/阶段 reactions、命令解析、provider/settings 表单 UI SHALL 全部由 im 服务持有与执行，其行为规格分别遵循 `channels`、`feishu-cards`、`permission-flow`、`dispatch-commands`、`provider-management` capability。阶段 reaction 的渲染 SHALL 由 im 前端基于从核心会话通道观察到的 `SessionInfo.phase` 变化驱动（seed→working→terminal 相位机、同 emoji 不重发、旧 reaction 尽力移除的 swap 语义，见 `feishu-reactions`）；core SHALL NOT 为 IM 通道渲染 reaction 或发送聊天向卡片。im SHALL 把交互蒸馏为核心通道请求（会话消息、审批决定、状态库变更），SHALL NOT 本地实现任何会话语义。**Deferred**：`[card]` 的截断/折叠/thinking 旋钮目前未被 im 卡片机消费（仅 `theme_color` 生效，其余走默认值）；设置域接管后随域刷新。

#### Scenario: 权限按钮点击走通道回传

- **WHEN** 用户点击 im 渲染的权限卡按钮
- **THEN** im 把决定经核心通道 `ApprovalAnswer` 回传，由核心路由给对应执行体

#### Scenario: 表单提交走状态库

- **WHEN** 用户提交 provider 表单
- **THEN** im 经通道状态库接口（StateMutation）持久化，反馈卡片如实回报成功或失败

#### Scenario: 相位变化触发 reaction

- **WHEN** im 前端观察到某会话的 `SessionInfo.phase` 由 seed 变为 working
- **THEN** im 在该会话的卡片消息上应用 `OnIt` reaction（同 emoji 不重复发 API）

### Requirement: 媒体解析与图片上传

im 服务 SHALL 负责媒体解析（拆分后只有 im 持有飞书凭据）：入站图片/文件经飞书 media API 下载到 `[media] download_dir`（大小上限校验、落盘），超过 `[media] max_file_size` 的附件 SHALL 被拒绝并以纯文本如实告知用户，下载失败 SHALL 如实反馈且不谎报已受理；解析成功的附件以本地引用（路径 + mime + 文件名）随会话请求上通道。出站图片展示（agent 产出图上传飞书）依赖执行体产出图片条目的上游能力（ACP 驱动尚无 image 变体），随 ACP 图片块能力另行立项，不在本 change 验收。

#### Scenario: 入站图片落地为本地附件

- **WHEN** 用户向飞书 bot 发送一张 2 MB 的 PNG 且 `max_file_size` 为 20 MB
- **THEN** im 下载该图到 `download_dir` 并识别 mime，会话请求携带其本地路径与 mime 上通道，模型最终收到该图内容

#### Scenario: 超限附件如实拒绝

- **WHEN** 用户发送一张超过 `max_file_size` 的文件
- **THEN** im 不发起下载投递，以纯文本告知大小限制，不声称已受理

### Requirement: 控制命令直连 watchdog

im 服务 SHALL 持有 watchdog 注入的控制凭据，把 `/upgrade`、`/rollback`、`/restart`、`/services`、`/system`、`/router`、`/webui` 等控制命令经 control RPC 直接发往 watchdog（不再绕道 core）；凭据缺失或调用失败时按 `dispatch-commands` 的反馈规格以纯文本如实回报。

#### Scenario: /system 由 im 直发 watchdog

- **WHEN** 用户向 im 服务的飞书通道发送 `/system`
- **THEN** im 直接向 watchdog control RPC 发起系统状态请求并把结果以纯文本回复

### Requirement: core 不可达时诚实降级

当核心通道不可达（socket 缺失、连接被拒、密钥被拒、连接断开），im SHALL 如实向 IM 用户呈现该状态与原因，SHALL NOT 把陈旧数据当作当前状态、SHALL NOT 谎报消息投递成功；核心恢复后 SHALL 自动重连并以新快照收敛。

#### Scenario: core 宕机期间的消息

- **WHEN** core 进程停止且用户向飞书 bot 发消息
- **THEN** im 如实回复核心不可达及原因，不声称已受理

#### Scenario: core 恢复后收敛

- **WHEN** core 重启完成
- **THEN** im 重连、取新快照，后续交互恢复正常且无需人工干预

### Requirement: 配置归属

IM 适配器配置（`[feishu]`）、渲染配置（`[card]`）、媒体配置（`[media]`）SHALL 由 im 服务解释并消费；core SHALL NOT 因这些配置建立任何 IM 连接。`[watchdog.im]` 与既有 `[watchdog.*]` 同形（enabled/host 类字段）。

#### Scenario: core 进程无视 feishu 配置

- **WHEN** 配置含完整 `[feishu]` 凭据但部署只运行 core
- **THEN** core 不建立飞书连接、不校验飞书 token，也不报配置错误

### Requirement: IM 前端渲染会话级 reaction

IM 服务的前端 SHALL 从核心会话通道的 `SessionInfo.phase` 推导并渲染会话级 reaction，遵循 `feishu-reactions` 的相位机、swap 与目标选择契约。core 进程 SHALL NOT 持有或发射 IM 向的 reaction 指令。

#### Scenario: 重启后由快照恢复相位

- **WHEN** core 重启且 im 服务持续运行
- **THEN** im 从重连后的会话快照重新取得各会话相位，并据此对齐卡片 reaction
