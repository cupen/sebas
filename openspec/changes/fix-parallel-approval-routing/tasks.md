# Tasks — fix-parallel-approval-routing

## 1. 失败测试钉根因

- [ ] 1.1 前端单测：审批面合并——构造「读模型 [A,B] + WS 推送 [B,A]」等 4 种到达顺序（GET先/WS先/交错/重复帧），断言每张卡的呈现内容（tool_name+args）与其提交时携带的 request_id 配对一致。当前实现应红（若不红，根因不在合并层，转 1.2 优先）。
- [ ] 1.2 后端单测：审批读模型枚举——同一待批集合两次枚举顺序稳定（排序键钉死）；泊车登记→枚举序与登记序一致。
- [ ] 1.3 dispatch 单测：两个挂起 hook_callback，乱序决策（先 Deny 后 Allow），断言各自结果路由到正确的请求；vendor 路径（cc-agent-sdk 补丁）在测试覆盖内。

## 2. 修复

- [ ] 2.1 审批面状态改 `request_id` 主键 map 合并；渲染 key 禁用 index（依 1.1 归因落点修复，可能同时涉及前端合并与后端枚举序两处）。
- [ ] 2.2 后端读模型枚举加稳定排序（与 1.2 的测试键一致）。
- [ ] 2.3 若 1.3 暴露 vendor 补丁路由偏差，修 vendor/cc-agent-sdk 补丁并记录补丁差异。

## 3. 回归与验收

- [ ] 3.1 全量 `cargo test` 过（dispatch / webui / 前端单测）；既有 parallel-permissions 套件不回归。
- [ ] 3.2 GUI 手测（沙箱，fake-claude `parallel`）：四轮——①顺序决策（Allow→Deny）②乱序决策（Deny→Allow）③刷新后顺序决策 ④刷新后乱序决策；每轮断言两工具结果不互换，截图留档。
- [ ] 3.3 单卡回归：ask 模式 `perm` Allow/Deny 两路径 GUI 手测不回归。
