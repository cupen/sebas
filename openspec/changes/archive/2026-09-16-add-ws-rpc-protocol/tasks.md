## 1. 协议帧与 codec 缝（后端）

- [x] 1.1 新增 webui crate 协议模块：`Frame` 三态类型（Request{id, method, params} / Response{id, result|error} / Notification{method, params}）与 `WsCodec` trait，JSON 实现（serde_json）；`cargo test` 下封套序列化往返单测（error/result 互斥、未知字段容忍）通过
- [x] 1.2 `events.rs` 的 `WebUiEvent` 组装改造：三条事件支路（session/permission/turn）统一包成 `Notification{method: 原 type, params: 原 payload}`；既有 ws 帧序列测试改为解封断言，`cargo test -p`（webui 测试目标）全绿

## 2. 服务端分发

- [x] 2.1 `ws_connection` 增加客户端 Text 帧解析支路：decode → Request 交 handler 注册表 → Response 回写；未知 method 回 `unknown_method` error 且连接保持；集成测试覆盖「未知 method 拒单不断连、后续 ping 正常」
- [x] 2.2 内置 `ping` handler（Request → `pong` Response）；ws 集成测试证明 id 关联往返；畸形帧（非 JSON、非三态）忽略不断连，测试覆盖

## 3. 前端协议层

- [x] 3.1 `api/ws.ts`：封套 encode/decode 函数对（JSON 实现）+ `request(method, params)`——id 自增、pending map 关联、超时（10s）拒付、未连接 `not_connected` 立即拒付、onclose 批量拒付；vitest 单测覆盖关联/超时/断线拒付/乱序 Response
- [x] 3.2 `ws.ts` 分发面平移：解封 Notification，`method` 当既有 `type` 分发（订阅 handler 与分发 key 不变），白名单平移为 method 集合、未知容忍；`shared-ws.ts` 注入 codec；既有 ws 单测更新后全绿，各视图订阅面零改动（`git diff` 仅限 ws.ts/shared-ws.ts）

## 4. 收口

- [x] 4.1 `testsuite-webui` 沙箱回归：登录后工作台实时事件（会话创建/turn 流/权限卡）经 Notification 正常到达，断线重连后 refetch 收敛；`invoke testsuite-webui-server` 冒烟通过（浏览器套件 71/72：唯一失败 session-roundtrip 为操作者在途 spec 自述的串行事件多播遗留，隔离跑 1.9s 绿；进程级 e2e `turn_appends_stream_over_ws` 解封断言后单跑绿）
- [x] 4.2 `openspec validate add-ws-rpc-protocol` 通过；COVERAGE.md 与 testsuite README 增补协议层测试覆盖说明（两文件在操作者在途脏集中，增补仅追加、不随本 change 过渡提交）
