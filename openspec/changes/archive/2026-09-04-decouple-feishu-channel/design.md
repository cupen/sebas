# design — decouple-feishu-channel

## Context

动机见 proposal.md。现状耦合的**本质**不是"核心调用了一个飞书库",而是**飞书类型被当作核心领域模型**:

- `SessionKey`(chat_id + thread_id)是 `core_channel` 协议、webui routes/backend、router maps/inbound 的会话标识;
- `FeishuIn`(文本/媒体/按钮/表单)是 router inbound 的输入事件类型;
- `Card`/`CardConfig` 是 router 对用户呈现和 webui `[card]` 配置段的形态;
- `FeishuEnvelope` 甚至进了 replay 与 core_channel 的命中。

`make-feishu-optional-webui-primary` 已经给了开关,但开关只是"可不用",类型边界没动——新 IM 接入仍会撞上飞书形状的 key/事件/卡片。

三个已确认的方向:抽中立通道抽象、允许改协议、webui 也进适配层。

## 核心链路

术语以 `openspec/glossary.md` 为准(core/router/webui/通道/适配器/会话/执行体/项目/工作台 等)。现状与目标的链路对照:

```
现状(飞书形状渗入核心):
  飞书 WS ──FeishuIn(SessionKey)──► router inbound ──► SessionMap ──► 执行体(ACP 桥 | 原生内核)
      ▲                                                                        │
      └── FeishuClient.send/update card ◄── render_accumulated_card(router 直接渲染)◄── turn 事件
  webui ──SessionKey──► core_channel(server dispatch)──► RouterHandle(同上)

目标(核心只依赖中立抽象):
  feishu adapter ──ChannelEvent──►┐
  web adapter    ──ChannelEvent──►┼──► router inbound ──► SessionMap(ChannelKey)──► 执行体(不变)
      ▲                           │                                        │
      └── 渠道渲染(API 调用)◄── ChannelCard(中立呈现)◄──────────────────────┘
  独立 webui ──ChannelKey──► core_channel(协议上移)──► 同一 SessionMap
```

要点:

- 链路的**会话层与执行体不动**:SessionMap、懒创建、turn 队列、ACP/原生双执行体全部保留;变的只是会话标识(SessionKey→ChannelKey)与事件/呈现的类型边界。
- **入站**:各通道适配器把渠道事件翻译为 `ChannelEvent` 交给 router;飞书的去重/聊天类型/mention 门禁留在 feishu adapter(feishu-bridge delta)。
- **出站**:router 只产 `ChannelCard`;卡片渲染(`feishu-cards` 全部规则)与 message_id 映射下沉到 feishu adapter。
- **进程外客户端**:core_channel 协议类型上移为 `ChannelKey`,独立 webui 的观察/驱动行为不变(core-session-channel delta)。

## Goals / Non-Goals

**Goals:**
- 核心定义中立通道抽象(`ChannelKey`/`ChannelEvent`/`ChannelCard`/适配器注册表),只依赖抽象,不依赖任何具体渠道。
- `SessionKey`/`FeishuIn`/`Card` 从核心领域模型剥离,飞书形状收敛到飞书适配器。
- webui 成为第一个非飞书通道实现;`sebas-router` 不再依赖 `sebas-feishu`。
- 所有对外行为(可选开关、webui 主控、双通道共享)保持,只换类型/边界。

**Non-Goals:**
- 不实现第二个适配器(新 IM/agent)——本 change 只铺抽象,落地 webui+飞书两个实现作验证。
- 不改状态库/存储 schema、不动 watchdog 控制面。
- 不重写飞书 WS 传输与 token 逻辑(那是飞书适配器内部,保留)。

## Decisions

### D1 中立类型的落点:新增 `sebas-channels` crate,而不是塞进核心 bin

`SessionKey`/`FeishuIn`/`Card` 现在从 `sebas-feishu` 被 `sebas-router`/`sebas-webui` 共同引用。中立类型需要被 `sebas-router`(领域)、`sebas-webui`(客户端)、`sebas-feishu`(适配器)、core bin(装配)四方共享。放 core bin 会让 router 依赖 bin;放 router 会让 webui/feishu 依赖具体实现。**新建 `sebas-channels` crate**(核心抽象 + 中立类型),core bin、sebas-router、sebas-webui、sebas-feishu 都依赖它。这与 `sebas-acp` 作为共享抽象 crate 的既有模式一致。

- 备选:放进 `sebas-router` 再让 webui/feishu 依赖 router → 会把 router 的内部(ex: `RouterHandle`)暴露给 webui,边界糊。
- 备选:放进 core bin → router 反向依赖 bin,编译环。排除。

```
        ┌──────────────────────────────────────────────┐
        │              sebas-channels (中立抽象)          │
        │  ChannelKey  ChannelEvent  ChannelCard        │
        │  ChannelAdapter trait   AdapterRegistry        │
        └───▲────────────▲───────────────▲───────────▲───┘
            │            │               │           │
     ┌──────┴───┐  ┌─────┴─────┐  ┌──────┴─────┐  ┌───┴────────┐
     │ core bin │  │ sebas-    │  │ sebas-     │  │ sebas-     │
     │ (run/    │  │ router    │  │ webui      │  │ feishu     │
     │ channel) │  │ (领域)     │  │ (web 通道)  │  │ (适配器)    │
     └──────────┘  └───────────┘  └────────────┘  └────────────┘
```

### D2 `ChannelKey` 的形状:channel 名 + 不透明引用,前缀语义废弃

```rust
pub struct ChannelKey {
    pub channel: ChannelName,   // "feishu" | "web" | 未来："whatsapp"...
    pub reference: String,      // 通道内不透明引用，如 "oc_x\0t1"，或 "w1"
}
```

- 核心**不解析** reference 内部结构(rust/serde 层面就是不透明 String),`web-*` / `oc_*` / `ou_*` 前缀特判全部移除,由 `ChannelName` 区分。
- `core_channel` 协议里 `SessionKey` → `ChannelKey` 是 **BREAKING**,同仓同步升级(webui 与 core 一起发),协议不对外承诺跨版本。
- 备选:保留 `chat_id`+`thread_id` 两字段只是重命名 → 会把飞书形状固化进协议,未来的 IM 不一定有 thread 概念。排除。

### D3 事件模型:`ChannelEvent` 取代 `FeishuIn` 进 router inbound

`FeishuIn::{Text, Media, ButtonCb, FormCb}` 的**变体形状**其实已经足够中立(text/media/button/form 是通用 IM 概念),问题是类型名和字段(特指 feishu message id / file_key)。所以:

```rust
pub enum ChannelEvent {
    Text { key: ChannelKey, text: String, reply_target: Option<String> },
    Media { key: ChannelKey, file_keys: Vec<String>, reply_target: Option<String> },
    ButtonCb { key: ChannelKey, action: String, payload: Option<String> },
    FormCb  { key: ChannelKey, form: String, values: BTreeMap<String, String> },
}
```

- `reply_target`(触发消息 id / 线程 root)是**通道中立元数据**——web 没有,飞书有,未来 IM 各有各的,放一个可有可无的 `Option` 而非一等字段。
- adapter 负责把飞书 callback 引用解析回会话(按钮 payload 里带回 source)，核心只看到 `ChannelEvent`。
- `session_boot` / `dispatch` / `replay` 从消费 `FeishuIn` 改为消费 `ChannelEvent`;`replay` 管线经 `sebas-channels` 中立类型,不引用飞书。

### D4 出站呈现:`ChannelCard` 取代 router 直接调 `FeishuClient`

现在 router 直接 `FeishuClient::send_card` / `update_card` + 卡片渲染(`render_accumulated_card`)全在核心侧。改成:

- `sebas-channels` 定义 `ChannelCard` —— 一次出站呈现的**中立累积模型**:标题/正文元素/思考面板/工具面板/用法/交互元素(按钮/表单/选择),带"冻结/更新/轮转"生命周期,不绑定任何 JSON schema。
- 核心产生 `ChannelCard` 更新,**推给 adapter**;adapter 把它渲染成飞书 card schema 2.0 JSON 并调 API(`feishu-cards` 的全部渲染规则:budget/轮转/思考折叠/截断/主题都进 adapter 的渲染实现)。
- router 不再 import `sebas_feishu::cards` / `client`;`[card]` 配置段由 adapter 解释。

### D5 适配器接口与注册表

```rust
pub trait ChannelAdapter: Send + Sync {
    fn channel_name(&self) -> ChannelName;
    fn spawn(&self) -> ...;                 // 启动自己的传输(飞书:WS 循环)
    fn shutdown(&self) -> ...;
    fn render(&self, key: &ChannelKey, card: &ChannelCard) -> ...; // 渲染+出站
}
```

- core bin 装配时,按配置实例化已启用的 adapter 填入 `AdapterRegistry`;`run.rs`/`ws_loop.rs` 不再硬编码 feishu 启动,改为"遍历注册表起 adapter"。
- 适配器**主动把 `ChannelEvent` 交给 core**(core 提供一个 `inbound_tx`),核心不回指 adapter 内部——这是单向依赖,避免循环。
- 配置 → 注册映射:`[feishu] enabled`/凭据 → 注册 feishu adapter;`web` 常驻注册。新 IM 只需新配置段 + 新 adapter 实现,核心路由/类型零改动。

### D6 协议迁移策略:类型先上移、行为后切

为控制风险,`core_channel` 一次改到位(协议类型换 `ChannelKey`),但**分两步落地**:

1. `sebas-channels` 建好,`ChannelKey`/`ChannelEvent`/`ChannelCard` 定义就位;
2. core 与 webui **同步**切到新类型,`SessionKey` 作为 `sebas-feishu` 内部类型保留(adapter 内用),不再出现在任何核心/协议接口。

不提供协议版本协商(同仓同发,见 D2)。`replay` 的 JSON 记录格式随 `ChannelEvent` 调整,旧 replay 文件失效——接受(开发阶段无生产数据)。**BREAKING** 标注在 proposal。

### D7 webui 作为 `web` 通道实现

webui 现在是"内嵌核心 + 经 `core_channel` 查询"的混合体。本次:

- `sebas-webui` 实现 `ChannelAdapter`(`web` 通道):把 `/api/sessions` 等读操作接到底层 `core_channel` 的 snapshot/events(已经是 `SessionBackend` 抽象),把"创建/发消息/关闭"接到底层 drive 方法。`session_backend` 的 trait 从飞书 `SessionKey` 换成 `ChannelKey`。
- webui **也**作为 adapter 注册进 core(在 core 进程内被装配),这使"web 会话"与"飞书会话"真正平级:都是 `ChannelKey{channel: web, ...}` / `{channel: feishu, ...}`,共享同一 `SessionManager`。
- webui 的 `routes.rs`/`models.rs` 的 `encode_session_key`(chat_id\0thread_id)改编码 `ChannelKey`;前缀特判移除。

### D8 `sebas-router` 只依赖 `sebas-channels`

- `router/mod.rs` 的 `update_card`/`reaction`/`reply` 方法签名改走 `ChannelCard` + `ChannelKey`,内部不再引用 `FeishuClient`/`Card`。
- `maps.rs` 的 message_id 映射(飞书 PATCH 需要)下沉到 adapter——adapter 维护"channel key → 飞书 message_id"对应;核心侧只留中立生命周期。
- `crud.rs` 从 `render_form_card`/`values_to_strings` 改走 `ChannelCard` 交互元素;`native_bridge` 的 default-native 语义不变,`SessionKey` 换 `ChannelKey`。
- `sebas-router/Cargo.toml` 移除 `sebas-feishu` 依赖,加 `sebas-channels`。这是"编译期保证不耦合"的关键证据。

### D9 配置:`[feishu]`/`[card]` 段保留,解释权移交

- `[feishu] enabled` 语义不变,只把"开关"变成"是否注册 feishu adapter"。
- `[card]`(feishu 渲染参数)由 webui 与 feishu 共享,但**解释权归 adapter**;核心不再读 `card.theme_color` 等渲染 knob。`config.rs` 的类型依赖从 `sebas_feishu::cards::CardConfig` 改为 `sebas-channels` 的等价类型。
- 这样将来第二个 adapter 可以有自己独立的渲染配置段,不用和 feishu 挤一个 `[card]`。

### D10 卡片引用簿记(`MsgIdMap`/`ReplyTargetMap`)保留在 router——中立 `CardRef` 语义

实施中发现:任务 3.3 原定"router 不再需要飞书 message_id 表",但把 `msgid`/`input_msg`/`help_card_msgid`/`reply_targets` 下沉到 dispatcher 意味着把卡片生命周期(冻结/轮转/perm 卡翻转)的**状态**搬出属主,dispatch 往返也要新增一倍回调面。重新审视后这些表的性质是:**按 `ChannelKey`/session_id 存不透明字符串引用**——正是 adapter trait 里 `CardRef` 的簿记形态。"当前 turn 的呈现实例是哪个引用"是任何原地更新型通道都需要的**中立**呈现生命周期状态,router 作为该生命周期的属主持有它;引用的**飞书含义**(PATCH endpoint、message_id、话题 root)只在 dispatcher/adapter 边界解释。router 全程不 import 飞书类型(`grep sebas_feishu sebas-router/` 为空),`channels` 能力"核心不把具体渠道 id 形状当一等领域概念"的场景由"引用不透明"满足。备选(把表下沉 dispatcher)记录在案:若未来第二个通道也需要原地更新,且出现双份簿记,再把 `CardRef` 簿记抽成 router 与 adapter 的共享 contract——本期不做。

## Risks / Trade-offs

- [协议 BREAKING: core_channel wire shape 变化,replay 旧文件失效] → 同仓同发、开发阶段接受;replay 格式随 `ChannelEvent` 调整,旧文件弃用。
- [重构面大: router/crud/maps/inbound/webui 的多文件类型替换] → 按"类型先上移、行为后切"分步,每步可单独编译/测试;`sebas-router` 移除 feishu 依赖是编译期闸门。
- [卡片渲染从 router 移到 adapter,`feishu-cards` 规则是否被忠实继承] → 渲染规则逐条落 spec(truncation/budget/rotation/thinking 全保留),adapter 用既有 `render_accumulated_card` 逻辑迁移,跑现有卡片测试套件验证。
- [webui 既内嵌又经通道的双形态在抽象下是否自洽] → `SessionBackend` trait 已经隔离了"core 操作",本次只把 trait 的 `SessionKey` 换 `ChannelKey`,行为面不变。

## Migration Plan

1. 建 `sebas-channels` crate,中立类型就位(编译期,不改行为)。
2. 核心与 webui 同步切 `ChannelKey`(core_channel 协议、router 领域、webui 调用)——同仓同发,一次合并。
3. feishu adapter:把 WS 生命周期、卡片渲染、reaction 映射迁进 adapter 实现 `ChannelAdapter`;`sebas-router` 移除 feishu 依赖,`run.rs`/`ws_loop.rs` 改注册表装配。
4. webui adapter:`web` 通道注册,`SessionKey`→`ChannelKey` 收尾,前缀特判移除。
5. 验证:现有 router/webui/replay 测试套件改夹具后全绿;`sebas-router` 不再依赖 `sebas-feishu`;手动启用 feishu + webui 双通道共享会话。
6. 回滚:同仓回滚同步切类型的那次合并(类型替换无持久化状态,回滚安全);不涉及状态库。

## Open Questions

- 新 IM 适配器的**第二个落地验证**是否纳入本 change(如加一个 `mock`/`terminal` adapter 端到端证明可插拔)——倾向留待独立 change,本 change 以 webui+飞书双实现作验证,避免 scope 膨胀。可安全后置,不改 spec/方案/任务。