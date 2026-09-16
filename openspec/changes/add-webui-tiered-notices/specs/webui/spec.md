## MODIFIED Requirements

### Requirement: 全局核心可达性横幅

app-shell SHALL 提供全局「核心不可达」fatal 通知：当 `/api/summary` 的 `reachability.ok` 为 false 时，在视口顶部居中通知层呈现驻留横幅，并对工作台整体施加锁定遮罩（交互与浏览一并锁住，工作台内的交互元素不可聚焦、不可激活）；横幅文案 SHALL 按 `reachability.kind` 分档呈现（`startup_failed` / `auth_rejected` / `disconnected` 各成一句），并保留 cause 原文；横幅呈现 role=alert 且 SHALL NOT 可手动关闭。锁定期间可达性轮询 SHALL 继续；横幅自身 SHALL 可交互；auth 门禁页（登录 / 首启设置）SHALL 不受锁定影响。可达性恢复后横幅 SHALL 消失、锁定 SHALL 解除，并 SHALL 弹出一条「核心已恢复」的 info 级通知。

#### Scenario: core 不可达时横幅出现

- **WHEN** core 进程停止或通道不可达，浏览器停留在工作台任意页面
- **THEN** fatal 横幅出现且文本按 `reachability.kind` 分档并含 cause 原文，工作台被锁定遮罩覆盖、其中交互元素不可聚焦不可激活

#### Scenario: core 恢复后横幅消失

- **WHEN** core 恢复服务且通道重新握手成功
- **THEN** 无需刷新页面，下一次可达性轮询后横幅消失、锁定解除，并出现「核心已恢复」info 通知

#### Scenario: auth 门禁页不受锁定影响

- **WHEN** 鉴权启用且操作者处于登录页或首启设置页时 core 不可达
- **THEN** 登录 / 设置流程照常可交互，不出现工作台锁定遮罩

## ADDED Requirements

### Requirement: 分级通知层

WebUI SHALL 提供唯一的视口级顶部居中通知层，按四级呈现全局通知：info（蓝色瞬时 toast，数秒自动消失）、warn（琥珀色：瞬时来源用自动消失 toast，持续状态用驻留横幅）、error（红色驻留 toast，须操作者手动关闭）、fatal（驻留横幅 + 锁定，语义见「全局核心可达性横幅」）。判级 SHALL 由前端按影响面裁定——应用瘫痪 = fatal、单个视图 / 能力不可用 = error、单个操作失败且可立即重试 = warn、无损状态提示 = info；HTTP 状态码只是信号、不直接定级。API 客户端 SHALL 对未豁免的请求失败自动按级弹出（操作失败类 → warn），而有内联错误呈现的表单调用点与带重试的列表加载 SHALL 豁免统一拦截、维持内联呈现；401 SHALL 走既有登录跳转、SHALL NOT 进入通知层。通知层 SHALL 满足：瞬时 toast 栈上限三条、超出挤掉最旧瞬时条（error 驻留条不参与挤占）；同一文案在去重窗口内 SHALL NOT 重复弹出；info / warn toast 可提前手动关闭；fatal 横幅不可手动关闭。既有「与服务器的连接已断开」横幅 SHALL 收编为本层的持续 warn 驻留横幅，旧实现 SHALL 移除。通知层与 settings 弹窗等浮层叠放时 SHALL 保持在上；窄屏 SHALL 退化为全宽贴顶。

#### Scenario: 四级形态可辨识

- **WHEN** info / warn / error / fatal 各级通知呈现
- **THEN** 四级在配色上可区分（info 蓝、warn 琥珀、error 红、fatal 红 + 锁定遮罩），且颜色不是唯一的信息通道（附图标与文案）

#### Scenario: API 操作失败自动弹 warn

- **WHEN** 一个未豁免的 API 调用因网络失败或服务端错误而失败（如保存请求超时）
- **THEN** 通知层弹出 warn 级失败提示并自动消失，该调用点无需自行处理全局呈现

#### Scenario: 内联错误点不双弹

- **WHEN** composer 提交、settings 保存或列表初始加载等有内联错误呈现的调用点失败
- **THEN** 错误维持内联呈现，通知层不重复弹出同一失败

#### Scenario: 驻留 error 须手动关闭

- **WHEN** 视图 / 能力级故障由视图显式上报为 error 级通知
- **THEN** 该通知不自动消失，操作者手动关闭后即移除

#### Scenario: 栈上限与去重

- **WHEN** 瞬时通知超过三条，或同一文案在去重窗口内重复触发
- **THEN** 超出时挤掉最旧的瞬时条，重复文案不产生第二条

#### Scenario: WS 断线收编为持续 warn 驻留横幅

- **WHEN** `/ws` 连接断开
- **THEN** 持续 warn 驻留横幅出现在通知层（旧 app-shell 横幅不再渲染），既有指数退避重连继续；重连成功后横幅消失并触发既有 `sebas:refetch` 刷新
