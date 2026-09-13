# Tasks: feishu-card-callback-observability

## 1. 回调到达性自检

- [x] 1.1 `sebas-im/src/frontend.rs`：`AtomicU64` 双计数（按钮卡发出数 / `card.action.trigger` 到达数）+ `AtomicBool` 告警闩锁；发出计数挂接全部交互卡发送出口（盘点并在汇报列明挂接点），到达计数挂 `on_button` 调用入口；发送后评估 `sent >= 3 && received == 0` 触发单次 WARN（文案含订阅配置检查指引与 `type=card` 标志形态）

## 2. 不可识别点击可见反馈

- [x] 2.1 `on_button` 两处静默早退（缺 request_id / 决定无法识别）改经 `ensure_message` 发「点击未识别」文本回执（附 decision 原始值缩略），INFO 日志保留，回执发送失败仅 warn 不上抛；单测覆盖：两早退路径各产生回执且文案含原因、合法点击不产生额外回执、回执失败不影响主流程

## 3. 计数与告警逻辑单测

- [x] 3.1 单测模拟计数序列：3 发 0 收触发 WARN、闩锁后不重复、收到回调后不再触发、2 发 0 收不触发（触发点与阈值语义按 design D2）

## 4. 收尾

- [x] 4.1 `cargo test -p sebas-im` 全绿、`cargo build` 通过；`openspec validate feishu-card-callback-observability --strict` 通过
- [x] 4.2 汇报验证边界：沙箱无真实飞书 ws，到达性仅验证计数逻辑；端到端人工复测归 bd sebas-033 跟进
