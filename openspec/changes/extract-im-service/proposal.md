# extract-im-service

## Why

core 进程今天既是会话核心又是 IM 前端：飞书适配器、卡片状态机、命令面、provider 表单（约 4-5k 行）长在 sebas-dispatch 里，出站 `Out` 枚举带满飞书 `message_id`/reaction 语义，`dispatch.rs` 把 ACP 执行与飞书发送混装。后果：core 二进制绑死 openlark 依赖；IM 故障域与核心同进程；将来接入第二个 IM 只能继续往 sebas-dispatch 里塞。webui 已验证「独立服务 + 核心会话通道」是干净的拆法——im 照此办理，成为 webui 的兄弟服务。

## What Changes

- 新增 `sebas-im` crate：IM 服务宿主——多 IM 适配器注册（feishu 第一个入驻，自 sebas-feishu 复用）、卡片渲染/更新/reactions、命令解析、provider/settings 表单 UI；随迁 sebas-dispatch 的 card_state / card_events / cards_ui / commands / crud / reactions。
- 新增 `sebas im` 独立服务（镜像 `sebas webui`）：经核心会话通道观察并驱动会话（Subscribe / Spawn / Message / ApprovalAnswer / Turns / StateMutation），watchdog 新增 im 托管服务与 `[watchdog.im]` 配置。
- **BREAKING** core 不再链接 sebas-feishu：`sebas core` 移除全部飞书装配（token 校验、hello/test 消息、`--test-msg`、`--dump-inbound` 随迁 im）；飞书接入只能以 `sebas im` 独立服务形态运行。
- 核心会话通道 additive 扩展：面向 IM 前端的会话型消息语义（未知 key 文本自动建会话、dormant 会话复活）、im 渲染所需内容流、**图片附件双向面**（入站消息携带本地附件引用、turn 内容携带图片条目）；旧客户端（webui）不受影响。
- **图片双向支持**（作为 agent 工作台，消息不止文字）：入站——im 凭凭据下载用户图片到 `[media] download_dir`，经通道以本地附件引用（路径 + mime）投递，执行体把图真正喂进模型（native 用既有 Image 块；ACP 走 `image` capability 协商，不支持时如实降级为路径标记）；出站——执行体产出的图片（如工具结果截图/图表）进 turn 内容图片条目，im 上传飞书换 `image_key` 后以卡片 img 元素展示。
- sebas-dispatch 瘦身为纯会话核心（SessionMap + ACP 泵 + 事件流）；`Out` 枚举中呈现类指令（SendCard / UpdateCard / React / AckMsg / PlainText / HelpText）随 UI 逻辑迁入 sebas-im。

## Capabilities

### New Capabilities

- `im-service`: 独立 IM 服务——多 IM 适配器宿主、经核心会话通道的会话前端面（观察 + 驱动 + 审批 + 表单）、watchdog 托管生命周期、`sebas im` CLI 与配置。

### Modified Capabilities

- `channels`: 适配器宿主从 core 进程改为 im 服务进程；core 不再注册具体 IM 适配器，中立模型（ChannelKey/ChannelEvent/ChannelCard）不变。
- `feishu-bridge`: 飞书桥宿主改为 sebas-im；入站解析/去重/门禁不变，出站呈现经会话通道而非进程内 `Out` 枚举；入站媒体从「只传 file key」改为「im 解析为可用附件」（拆分后只有 im 持飞书凭据）。
- `feishu-cards`: 中立呈现模型的产出方从 router 改为 sebas-im；卡片渲染行为对用户不变；新增图片元素渲染（上传换 `image_key`）。
- `core-session-channel`: 新增 IM 前端所需的会话型消息与内容流消息（additive，旧客户端兼容）；消息请求增可选附件（本地路径 + mime）、turn 内容增图片条目。
- `dispatch-commands`: 命令解析与处理的宿主改为 sebas-im；控制类命令由 im 直连 watchdog control RPC。
- `feishu-option`: `[feishu] enabled` 开关决定 im 服务中飞书适配器的注册；core 彻底不含飞书。
- `watchdog`: 新增 im 托管服务（ServiceName、`[watchdog.im]`、服务生命周期与健康探测）。

## Impact

- 代码：新 crate `sebas-im`；`src/`（run.rs、dispatch.rs、cli.rs、新增 im_cmd、watchdog、core_channel/protocol.rs additive）；`sebas-dispatch` 移除 IM UX 模块；`sebas-feishu` 保留（宿主改为 im；`media.rs` 下载/上传从死代码转为真实链路）；workspace 成员更新。
- 部署：watchdog 服务清单、ansible 剧本、升级/回滚范围、sandbox 调试食谱新增 im 进程；`[media] download_dir` 成为 im 的必配存储面。
- 前置依赖：`wire-webui-sebas-agent-e2e` 先落地（两 change 动同一片 core_channel / run.rs）。
- 行为保持：飞书侧用户可见行为（卡片、审批、命令、表单、reactions）不变；新增图片双向能力；单二进制调试形态（`run --webui --debug`）不再含飞书。

## Non-goals

- 不实现第二个 IM 适配器（只保证 seam 与宿主就绪）。
- 不改 webui 工作台行为与鉴权；webui 侧图片面（composer 传图、turn 流渲染图片）另立项——通道协议按附件就位，webui 后续接入零协议改动。
- 不动 gateway；图片不经 gateway 转发。
- 不做 im 服务多实例 / 水平扩展。
- 出站图片展示（agent 产出图上传飞书）随 ACP 驱动图片块能力另行立项：实现期核实 `AcpEvent` 尚无 image 变体，agent 侧没有产出图的数据源（design D8 记录了该裁决）。
- 单二进制部署形态下 `sebas` 主二进制同时承载 `core` 与 `im` 子命令，链接期共享 IM 实现代码；隔离口径为**进程行为**（`sebas core` 不建立任何 IM 连接、不注册适配器），不是链接期隔离。
