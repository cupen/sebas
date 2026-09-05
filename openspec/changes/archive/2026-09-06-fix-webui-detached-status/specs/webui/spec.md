## ADDED Requirements

### Requirement: Provider status parity across deployment forms

The WebUI's provider-derived surfaces (`GET /api/settings` 的 gateway 段、
`GET /api/gateway`、`GET /api/about` 的 provider 计数，及 composer 的
provider 标签）SHALL 在 `run --webui` 与 `sebas webui` 两种部署形态下，对同一
配置呈现一致且真实的 provider 状态。detached 形态 SHALL NOT 以空占位
（`GatewayInfo` 缺省值）作为最终数据源：provider 列表 SHALL 来自 webui 可达的
provider 真源（状态库），gateway 静态事实（listen、debug、has_auth）SHALL
来自配置解析。当 provider 真源不可用时，响应 SHALL 如实标注不可用，而不是
报告"未配置 provider"。

#### Scenario: detached 与 in-process 的 provider 标签一致

- **WHEN** 同一份含已注册 provider 的配置分别以 `run --webui` 与
  `sebas webui`（core 经通道在跑）启动，浏览器打开工作台 composer
- **THEN** 两者的 provider 标签显示相同的 provider 名，而非 detached 侧显示
  "no provider configured"

#### Scenario: detached 反映运行期 provider 变更

- **WHEN** 操作员经 gateway admin API 新增或改名 provider 后刷新 detached
  WebUI 的 settings
- **THEN** 响应中的 provider 集合反映该变更，无需重启 webui 进程

#### Scenario: provider 真源不可用时如实上报

- **WHEN** detached webui 无法从状态库读取 provider 数据
- **THEN** `/api/settings` 的 gateway 段携带可辨识的"不可用"指示，而不是把空
  集合冒充"未配置"

### Requirement: Honest session rejection causes

会话创建/驱动被拒时呈现给操作员的原因 SHALL 区分"核心（通道）不可达"与
"目标执行体不可用"：执行体侧的拒绝（如 native 缺 provider 凭据）SHALL 在
文案中指名执行体与真实原因，SHALL NOT 复用"核心不可达"（unreachable）字样；
仅当请求确实无法送达会话权威（通道断开、核心不在）时才呈现不可达语义。

#### Scenario: native 缺凭据的拒绝不再误报核心不可达

- **WHEN** 核心在运行且通道连通，但 native 执行体未配置 provider 凭据，
  客户端以 `backend: "native"` 请求创建会话
- **THEN** 拒绝文案指名 native 执行体与缺凭据原因，且不包含"核心不可达"

#### Scenario: 通道断开仍呈现不可达

- **WHEN** 核心未运行（通道不可达）时请求创建会话
- **THEN** 拒绝呈现核心不可达语义及其 cause
