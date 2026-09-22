## MODIFIED Requirements

### Requirement: 分级通知层

WebUI SHALL 提供唯一的视口级顶部居中通知层，按四级呈现全局通知：info（蓝色瞬
时 toast，数秒自动消失）、warn（琥珀色：瞬时来源用自动消失 toast，持续状态用
驻留横幅）、error（红色瞬时 toast，默认 8 秒自动消失；调用点显式指定驻留
（duration=0）时仍为驻留条、须操作者手动关闭）、fatal（驻留横幅 + 锁定，语义
见「全局核心可达性横幅」）。判级 SHALL 由前端按影响面裁定——应用瘫痪 =
fatal、单个视图 / 能力不可用 = error、单个操作失败且可立即重试 = warn、无损
状态提示 = info；HTTP 状态码只是信号、不直接定级。API 客户端 SHALL 对未豁免
的请求失败自动按级弹出（操作失败类 → warn），而有内联错误呈现的表单调用点与
带重试的列表加载 SHALL 豁免统一拦截、维持内联呈现；401 SHALL 走既有登录跳
转、SHALL NOT 进入通知层。通知层 SHALL 满足：瞬时 toast 栈上限三条、超出挤
掉最旧瞬时条（显式驻留条不参与挤占；error 默认条按瞬时条参与挤占）；同一文
案在去重窗口内 SHALL NOT 重复弹出；info / warn / error toast 可提前手动关
闭；fatal 横幅不可手动关闭。既有「与服务器的连接已断开」横幅 SHALL 收编为本
层的持续 warn 驻留横幅，旧实现 SHALL 移除。通知层与 settings 弹窗等浮层叠放
时 SHALL 保持在上；窄屏 SHALL 退化为全宽贴顶。

#### Scenario: 四级形态可辨识

- **WHEN** info / warn / error / fatal 各级通知呈现
- **THEN** 四级在配色上可区分（info 蓝、warn 琥珀、error 红、fatal 红 + 锁定
  遮罩），且颜色不是唯一的信息通道（附图标与文案）

#### Scenario: API 操作失败自动弹 warn

- **WHEN** 一个未豁免的 API 调用因网络失败或服务端错误而失败（如保存请求超时）
- **THEN** 通知层弹出 warn 级失败提示并自动消失，该调用点无需自行处理全局呈现

#### Scenario: 内联错误点不双弹

- **WHEN** composer 提交、settings 保存或列表初始加载等有内联错误呈现的调用点失败
- **THEN** 错误维持内联呈现，通知层不重复弹出同一失败

#### Scenario: error 默认自动消失且参与挤占

- **WHEN** 视图 / 能力级故障由视图显式上报为 error 级通知，且未显式指定驻留
- **THEN** 该通知默认 8 秒自动消失，并作为瞬时条参与三条栈上限的挤占（持续型
  故障由 fatal 横幅槽位承载，不依赖 error toast 驻留）

#### Scenario: 驻留 error 须手动关闭

- **WHEN** 调用点显式以 duration=0 上报 error 级通知（要求驻留）
- **THEN** 该通知不自动消失、不参与瞬时栈挤占，操作者手动关闭后即移除

#### Scenario: 栈上限与去重

- **WHEN** 瞬时通知超过三条，或同一文案在去重窗口内重复触发
- **THEN** 超出时挤掉最旧的瞬时条，重复文案不产生第二条

#### Scenario: WS 断线收编为持续 warn 驻留横幅

- **WHEN** `/ws` 连接断开
- **THEN** 持续 warn 驻留横幅出现在通知层（旧 app-shell 横幅不再渲染），既有
  指数退避重连继续；重连成功后横幅消失并触发既有 `sebas:refetch` 刷新

#### Scenario: 鉴权拒绝型断开不亮断连横幅

- **WHEN** 登录态下 `/ws` 因鉴权被拒而断开（而非网络断开），且重连退避在运行
- **THEN** 不弹出「服务器断开」warn 横幅（避免误导为服务故障）；页面就绪即重
  连并清空退避，连接恢复后一切照常
