## Context

第三轮 GUI 黑盒验收（见 proposal Why）在 `sebas-webui/frontend`（Lit + Web Awesome 组件栈）
发现 2 个交互失效、1 个呈现缺失与一批 P3 瑕疵。后端无涉：权限等待的 rail 状态、会话行名
实时翻转、reload 后权限卡重建均正常工作，说明 WS 推送链路与读模型本身健康，缺陷集中在
前端组件对推送/激活事件的处理层。

测试环境事实（复现依据）：
- 创建按钮被吞 2/2 复现，且仅发生在「操作过 Agent 下拉（wa-select）」之后；未动下拉的
  路径一次成功。Playwright 合成点击对 WA 组件普遍报可交互性超时，坐标真实点击可命中
  组件，但被吞的那一次点击无任何效果。
- 权限卡空悬复现 1 次：rail「等待」+ 提交钮红色（turn in flight）但审查卡未出现；
  reload 后卡由读模型重建成功（`agent-workbench`「Parked remote approvals surface in
  the workbench」的重建半边工作，推送半边丢失）。当次会话此前经历过
  allow-for-session 切 `auto` → 手动切回 `ask` 的模式迁移。

## Goals / Non-Goals

Goals：三个缺陷在 fake-claude 沙箱按原复现路径回归通过；P3 瑕疵同批清零；
不回归既有 e2e / acceptance 套件。

Non-Goals：不改后端与 wire 协议；不重写权限卡为通用通知系统；不引入新的
状态管理框架；不处理本轮未测区域（见 proposal Non-goals）。

## Decisions

1. **创建首次激活：先复现定位 overlay 残留，再选最小修复面。**
   首选怀疑 wa-select 浮层收起后遗留的透明 active-overlay 在按钮上方拦截命中。
   实现时先在复现路径用 hit-target 探测确认；若坐实，修复选「浮层关闭时确保
   overlay 卸载/失去拦截」（组件层清理或事件时序修正），而非绕开组件改用原生
   button 重写对话框。理由：原生重写面大且破坏 WA 视觉一致性；overlay 残留是
   通用缺陷，修在源头同时惠及其它 wa-select 使用点（模式切换下拉同构）。
   备选（若 overlay 假设不成立）：改为 dialog 级 form-submit 语义，让 Enter/点击
   汇聚到同一 submit handler，绕开单按钮命中问题。

2. **创建忙态与防重复提交：dialog 单一 in-flight 标志。**
   提交 handler 入口统一 guard：in-flight 时忽略后续激活；确认控件呈现
   busy（禁用 + 指示）。不用请求层去重（键控复杂且治标）。

3. **权限卡：渲染收敛为读模型单一入口，推送只写数据。**
   实测重建路径（reload/open 从读模型渲染）可靠，推送路径（实时增量渲染）丢卡。
   把审查卡渲染收敛为「读模型 store → 渲染」单一入口：WS 推送到达时只更新 store
   并触发同一渲染入口，不再有与重建并行的独立推送渲染分支。这同时天然满足
   「merge with pushes by request_id，不重复渲染、已决策不复活」。不选「修补推送
   分支的渲染调用」——那会保留双入口，回归面重复出现。
   模式迁移（auto→ask）作为必含回归用例覆盖。

4. **未读徽章：rail 渲染时按 per-browser 锚点计算。**
   `session-unread-badge` 的锚点/seen 边界语义不变；缺陷在非聚焦会话行未按锚点
   呈现计数。修 rail 行渲染从锚点推导徽章（计数 + 行强调），推送到达与切换焦点
   两条路径共用同一推导。不改锚点存储。

5. **P3 批量项逐一小修**：wa-select 选项文案换行 → 放宽选项面板最小宽度并允许
   单行省略；深链/刷新直达会话时项目标题 → 从会话归属项目绑定而非依赖 rail
   选中态；About Rust toolchain 空值 → 构建期注入失败时显示「未知」而非空串；
   路径 `\` `/` 混用 → 展示前统一规范化；Services 页内联代码断行 → `sebas run`
   作为整体不断行。

## Risks / Trade-offs

- [wa-select overlay 修复牵连所有使用点（模式切换、agent 下拉、执行节点选择）]
  → 修复后对三个使用点各跑一次冒烟（打开/选择/收起后立即点击其它控件）。
- [权限卡渲染收敛改动涉及推送与重建两条既有路径] → 先补一条「推送即时渲染」的
  前端单测固定期望行为，再动结构；allow-for-session→切回 ask 的模式迁移作为
  集成回归用例。
- [未读徽章推导若把「等待/进行中」会话误标未读] → 遵循既有 spec 场景：仅对
  「新回复条目到达且非聚焦」计数；聚焦会话与流式底部跟读不产生徽章。

## Migration Plan

纯前端修复，无数据迁移。部署即生效；回滚即 revert 提交。

## Open Questions

（无——overlay 根因若与假设不符，按决策 1 的备选路径走，不影响 spec 与任务拆分。）

## 附录：根因定位（实现期补充，已被浏览器回归两次修订）

round3 实现轮先做了代码层定位（WA chunk 静态分析，供对照保留在缺陷 1 小节），
随后两轮**真实浏览器回归逐层证伪了当时的假设**——overlay 残留假设、
「wa-button 激活怪癖」假设都被否掉，第三轮把根因钉在 **popover dismiss 吞
click** 上并再次更换修复面。全过程记入缺陷 1 小节。

### 缺陷 1：创建按钮首次激活被吞

**结论（round3 第三轮修订）：根因是 Popover API 的 dismiss 兼容语义——
popover 的 hidePopover() 在一条指针事件序列（pointerdown → pointerup →
click）期间被调用时，浏览器把该序列后续的 click 吞掉（防止「关闭弹层的这
一下点击」同时击穿到下层控件）。修复面：摘层不再调用 hidePopover、change
后立即清残留、pointerdown 只检测且动作延后；原生 button + Enter 提交与
300ms 同窗去重保留。**

**三轮证据链**（主 agent 真实浏览器回归，反复验证）：

1. 复现序列不变：new-session 弹窗 → wa-select 选 agent → 单击创建 → 后端无
   创建请求；第二次单击必成功（换原生 button 后依旧）。
2. 吞点击瞬间穿透 shadow DOM 的 elementFromPoint 命中链完全正常直达 button
   （无元素遮挡、无 active wa-popup 属性残留）——**「浮层残留挡路」假设
   被证伪**（第一轮 rescue 的摘层目标在真实复现里根本不出现），也解释了
   第二轮换原生 button 为何无效：吞点击发生在 popover 层，命中测试本来就
   到得了按钮，与控件实现无关。
3. 同一弹窗会话内「取消」第一次点击就有效；「先点弹窗其它区域再点创建」
   成功（第一次点击被『牺牲』在别处）——被吞的是「操作过下拉之后的第一个
   指针序列的 click」，不是特定控件。

**前两轮修复失败的原因**：

- 第一轮（wa-select-rescue：pointerdown 捕获**同步**摘层，含 hidePopover()
  调用）：摘层目标不存在使它多半空转，但一旦命中残留，它自己的
  hidePopover() 恰好在指针序列内被调用——按 dismiss 语义亲手制造吞点击，
  与 WA 自身的异步收尾同罪。
- 第二轮（确认/取消换原生 button + dialog 级 Enter 提交）：激活面确实干净
  了，但吞点击的来源在 popover 层（见上），按钮换什么实现都躲不开。

**hidePopover 在指针序列期间落地的两个来源**：

- WA 收起链是异步的多步链：`open=false` → `animateWithClass(popup, "hide")`
  → `listbox.hidden` → `popup.active=false` → wa-popup `updated()` →
  `stop()` → `hidePopover()`；且 `animateWithClass` 存在返回永不 settle 之
  Promise 的早退分支——hidePopover 可能拖到下一次指针交互期间才落地；
- 前两轮 rescue 自己的 pointerdown 捕获同步 hidePopover（见上）。

**本轮修复面（wa-select-rescue 第三版 + dialog 收尾，三条缺一不可）**：

- **第二轮措施保留**：确认/取消控件维持**原生 `<button>`**（视觉用 WA 主题
  token 在组件样式里复刻 brand/plain 两档——文档级 `native.css` 进不了
  shadow DOM，就地等价实现）+ **dialog 级表单提交语义**（Enter 在弹窗面上
  触发确认，与点击汇聚同一 `confirm()` 入口；按钮与 wa-select 自留键面让
  路，不双发不误发）。它们清干净了激活面，虽非根因解药，仍是修复的组成部分。
- **摘层不再调用 hidePopover()**：对残留 popup 容器（wa-popup 影子里的
  `div[popover]`，top-layer 真身）直接 `removeAttribute('popover')`——脱离
  popover 注册（规范的属性变更路径同步退出 top layer，不走 hidePopover
  入口、不触发 dismiss 语义）——再照常压灭视觉态（`active=false` +
  `listbox.hidden`）。WA 自身会在 active 翻 false 时经 stop() 调
  hidePopover，容器已摘注册后那条调用只会抛 NotSupportedError：摘层时同步
  把容器上的 hidePopover 短路成 no-op，0ms 后（WA 收尾微任务跑完）恢复原型
  方法与 popover 属性，保住下一轮 open 的 showPopover 路径——select 完全
  可复用。模块源码里不再有任何 hidePopover 调用（源码读回契约钉死）。
- **挂载点提前**：document 级 `change` 捕获监听（change 事件 composed，沿
  composedPath 找 wa-select），收起链一启动就经 0ms 定时器清掉该 select 的
  残留——把残留消灭在下一次用户点击**之前**，而不是等下一次 pointerdown。
- **pointerdown 捕获兜底只做「检测 + 记录」**：命中残留也只把清理动作排进
  0ms 定时器，当前事件分发完成后才动 popover 状态——绝不在指针序列内同步
  改 popover 状态（前两轮的教训）。清理落地时重查残留判定（用户已把下拉
  重新打开 = 正常交互，不碰）。
- **300ms 同窗去重**（new-session-dialog confirm 入口，钦定的最后一道网）：
  busy 由 rail 异步置位，快速二次激活在 busy 翻转前仍可能双发
  dialog-confirm——同一弹窗会话内 300ms 内的第二次激活忽略，重开弹窗窗口
  归零。除此之外不再加任何吞点击相关 hack，保持行为简单。

**代码层分析（保留供对照）**：静态分析 `@awesome.me/webawesome` chunk 得知
wa-select 收起是一条多步异步链，`animateWithClass` 的早退分支返回永不
settle 的 Promise——该缺陷形态真实存在于 WA 源码，是「残留」出现的土壤；
但把它当成唯一根因（顶层浮层挡路）方向错了：真正的吞点击在 popover dismiss
语义，与残留是否可见无关。

### 缺陷 2：权限卡推送不渲染、reload 才重建

推送路径（`permission.requested` 帧 → 直接 append `cards`）与读模型路径
（`pullApprovals` → append `cards`）是两个并行写入口，合并语义各写一份；
推送半边丢帧（广播 lag / 组件重挂载窗口）后没有任何补偿，唯一出路是
reload 触发重建。已按决策 3 收敛：`mergeRows()` 成为唯一 store 入口
（去重 + 已决墓碑 + 归一都在这一处），推送只把帧归一为 ApprovalRow 写
store；并新增 `sessionPhase` 对账——相位帧说 `waiting` 而 store 无待决卡
（= 推送丢帧）时经同一入口重取读模型，卡就地补齐，不再依赖 reload。

### 缺陷 3：非聚焦会话无未读徽章

rail 行的锚点推导（`unreadCount` + 行强调 + 相位帧就地补丁）本身健全且
有单测；缺的是**锚的建立**：`writeFocusAnchor` 只在 rail 点击切换
（`openSession`）时写，创建（服务端 set_focus 直达焦点）、深链、恢复聚焦
三条路径永远不写锚——而无锚会话按 spec（session-unread-badge「first
visit shows no unread」）读作 fully-read，于是这些会话之后的非聚焦新回复
推不出徽章。修复为两处一次性立锚：创建成功即以 0 段立锚（rail）；
聚焦会话详情到达且文档可见时锚缺失则以真实 `msg_count` 立锚（dashboard，
`loadFocused`）。锚已存在绝不回写，流式/隐藏 tab 的推进语义仍归
transcript，单调由游标模块保证。
