# decouple-feishu-channel

## Why

飞书接入虽已有 `[feishu] enabled` 开关,但开关只是"可不用",核心链路本身仍被飞书形状的类型绑架:`SessionKey`(chat_id+thread_id)、`FeishuIn`(文本/媒体/按钮/表单)、`Card` 直接被当作核心领域模型,渗进 `core_channel` 协议、router 的 maps/inbound/crud、webui 的 routes/backend、乃至 replay。任何新 IM 或 agent 客户端接入,都必然撞上"飞书形状的会话 key / 输入事件 / 呈现卡片",被迫在核心里继续堆飞书特例。

## What Changes

- **核心引入中立"通道"抽象**(新 capability `channels`):会话标识 `ChannelKey`、入站事件 `ChannelEvent`、出站呈现 `ChannelCard` 成为核心领域模型;核心只依赖抽象,不再依赖任何具体渠道。
- **BREAKING**:`core_channel` 协议与类型随之上移——`SessionKey` → `ChannelKey`,协议 wire shape 调整;webui 与 core 同仓同步升级,协议不对外承诺。
- **飞书降为可选通道实现**:`sebas-feishu` 实现通道抽象(WS 出入站、卡片/反应映射、token/重试),不再被 `sebas-router` 直接依赖;`sebas-router` 只依赖通道抽象。
- **webui 成为第一个非飞书通道实现**:webui 卡片、路由、会话后端改走抽象;现有 `feishu-bridge`(WS 生命周期)保留为飞书通道的传输实现,`feishu-cards`/`feishu-reactions` 的呈现语义移入抽象或飞书适配层。
- **可选开关语义保留**:`[feishu] enabled`、webui 主控部署形态不变,只把"开关的对象"从"硬编码的飞书"换成"适配器注册表"。
- 旧状态文件 / 状态库(sebas.db)不涉及;本 change 只动通道/桥/呈现的类型边界,不碰状态存储。

## Capabilities

### New Capabilities

- `channels`: 核心的中立通道抽象——`ChannelKey` 会话标识、`ChannelEvent` 入站事件(文本/媒体/按钮/表单)、`ChannelCard` 出站呈现、`ChannelAdapter` 实现接口;核心进程内只依赖此抽象,具体渠道(飞书/webui/未来 IM)以适配器形式接入。

### Modified Capabilities

- `feishu-bridge`: WS 出入站实现改为实现 `channels` 抽象;去线程化/直连核心的形态收敛为"适配器之一",不再承担核心领域模型职责。
- `core-session-channel`: 协议类型从 `SessionKey` 上移为中立 `ChannelKey`;wire shape 随抽象调整;**BREAKING**。
- `feishu-cards`: 卡片呈现语义从"核心直接调用"改为"核心经抽象出站 → 飞书适配器渲染";`render_accumulated_card` 等从 router 依赖中剥离。
- `feishu-option`: 可开关的语义保留,但开关对象从"硬编码飞书"改为"适配器注册表"。
- `session-lifecycle`: 会话身份从 (chat_id, thread_id) 二元组改为中立 `ChannelKey`(通道名 + 不透明引用);懒创建/队列/清理等生命周期语义不变。

## Impact

- **移除/改依赖**:`sebas-router` 不再依赖 `sebas-feishu`;`sebas-webui` 依赖收敛为通道抽象 + 可选适配器。
- **核心模块**:`src/core_channel/protocol.rs`(类型上移)、`src/config.rs`(card/feishu 段归位)、`src/run.rs`/`ws_loop.rs`(装配改为适配器注入)。
- **router**:`sebas-router/src/router/{inbound,mod,maps}.rs`、`crud.rs`、`native_bridge.rs` 从飞书类型改为抽象类型。
- **webui**:`routes.rs`/`session_backend.rs`/`server.rs`/`models.rs` 改走抽象;`session_backend` 成为 `channels` 的一个实现。
- **新增依赖**:无(抽象放核心 crate 或独立 crate);`sebas-feishu` 保留为可选适配器实现。

## Non-goals

- 不做飞书数据迁移、不改状态库(sebas.db)与已有存储 schema。
- 不实现任何新 IM/agent 客户端接入——本 change 只铺中立抽象,不落地第二个适配器。
- 不改变 `[feishu] enabled`、webui 主控、双通道共享会话的**外部行为**;只换类型/边界。
- 不重写飞书 WS 传输与 token 逻辑本身。