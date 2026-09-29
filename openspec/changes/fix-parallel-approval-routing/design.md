# Design — fix-parallel-approval-routing

## Context

第三轮 GUI QA 实测（w2 检查项 7，四轮 `parallel`）：

- 两张卡确实并发呈现（round2 已修的渲染面正常）。
- 决策路由 2/4 轮颠倒；关键线索——**刷新页面后挂起卡的显示顺序会翻转**；显示顺序与内部待批队列一致的两轮路由正确，不一致的两轮「点哪张卡都判给另一张」。
- 单卡路径全程正确 → 缺陷只在 ≥2 张挂起卡的配对/路由。

wire 现状：前端 `answerPermission(requestId, decision)` 已携带 request_id（`api/client.ts`），POST body 形如 `{request_id, decision:{...}}`——**前端调用形状不缺字段**，根因在「卡片内容 ↔ request_id」的配对环节或后端路由环节。

## 根因假设（实现首步以失败测试钉死，再修）

1. **主嫌疑——审批面合并错配**：审批面由「读模型 GET /api/sessions/{key}/approvals」与「WS 推送」两路合并（`PendingApprovalInfo` 注释自称「按 request_id 幂等合并」）。若合并实现按**数组位置**或「整表替换时机」处理，两路枚举顺序不一致时（后端读模型枚举序 vs WS 事件序），卡片内容与 request_id 错配 → 点击的是「工具 A 的内容 + 工具 B 的 id」→ 后端如实路由到 B。这同时解释「刷新后顺序翻转才错」：刷新改变了两路的相对顺序。
2. **次嫌疑——后端枚举序不稳定**：读模型/泊车登记的枚举无稳定排序（HashMap 遍历序），同集合两次 GET 顺序不同；若前端任何位置用 index 作 key 或增量合并，错配概率上升。
3. **再次嫌疑——vendor/cc-agent-sdk 0.1.7 补丁面**：round2 为并行卡改过「克隆回调后先放锁再 await」；若 hook_callback 与 PermissionReply 的关联键在补丁路径上有偏差（如按到达序弹队），会出现「队列序路由」。若 1/2 排除，沿此深挖。

## 方案

- 配对修复：审批面状态一律以 `request_id` 为主键做 map 合并（list 仅作渲染投影）；WS 与读模型两路无论谁先到、顺序如何，同一 request_id 只存在一张卡；渲染层 key 用 request_id，禁止 index key。
- 顺序确定性：读模型枚举加稳定排序（按登记序/时间戳/Id 排序任选其一，写进单测）；前端渲染按同一排序键输出。
- 路由回归钉：dispatch 侧单测——两个挂起请求，先 deny 后 allow 乱序决策，断言各自 hook_callback 收到正确结果；webui 侧单测——两路合并在 4 种到达顺序（GET先/WS先/交错/重复）下卡片内容与 id 配对一致。
- 验收路径：`cargo test`（dispatch + webui 单测）→ GUI 手测 `parallel` 四轮脚本（顺序决策 / 乱序决策 / 刷新后决策 / 刷新后乱序）——脚本与判定标准进 tasks。

## 被否备选

- 「后端忽略 request_id、按队列 FIFO 弹出 + 前端保证顺序」——否：把正确性押在两端顺序一致上，正是本缺陷形态；request_id 本就在 wire 上，精确路由是既有语义（stale-click 处理已按 id 判重）。
- 「只修前端合并、不动后端排序」——否：枚举序不稳定是持续错配源，单测可钉的成本极低，一并收口。

## 假设

- 挂起卡排序键的具体选择（登记序 vs id 字典序）是实现自由度，spec 只要求稳定。
- 远端（feishu）审批面若复用同一合并代码则自动受益；不复用则不在本期验证范围。
