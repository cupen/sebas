## MODIFIED Requirements

### Requirement: 鉴权与连接姿态不变

`/ws` 的鉴权拦截（auth 开启时升级前拒绝）与既有连接生命周期（协议级 ping/pong 心跳、断线重连由客户端承担）SHALL 不因协议层改变。

未认证状态下客户端 SHALL NOT 对 `/ws` 发起重试风暴：登录前（或会话失效后）的连接尝试 SHALL 静默抑制或以长退避重试，页面 console SHALL NOT 出现每次加载多条的 401 升级失败噪音。认证成功后恢复正常连接姿态。

#### Scenario: 未认证升级仍被拒

- **WHEN** auth 开启且无会话凭据
- **THEN** `/ws` 升级照旧被拒，协议层不参与

#### Scenario: 登录前无重试噪音

- **WHEN** auth 开启且未登录的操作者打开任意页面
- **THEN** 页面对 `/ws` 至多做一次静默尝试（或长退避重试），console 不出现反复 401 失败刷屏
