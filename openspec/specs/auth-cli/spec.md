# auth-cli Specification

## Purpose
`sebas auth` 组命令是 WebUI 账户体系在 CLI 侧的管理面：直接操作 auth.db
用户库完成建户、改密与列表，不依赖运行中的 sebas 实例，供部署引导、
沙箱装配与密码轮换使用。

## Requirements

### Requirement: auth 组命令形态

`sebas` SHALL 提供 `auth` 组命令，含三个子命令：`add`（建户）、
`passwd`（改密）、`list`（只读列表）。目标用户名 SHALL 为位置参数。
子命令语义 SHALL 按动词拆分——`add` 只建户、`passwd` 只改密，
SHALL NOT 提供 create-or-update 一体形态。

#### Scenario: add 建户成功

- **WHEN** 库中不存在用户 alice（大小写不敏感比对）时执行
  `printf '%s' 'pw' | sebas auth add alice`
- **THEN** 用户按缺省角色规则创建成功，输出确认信息与用户库路径

#### Scenario: add 同名已存在报错

- **WHEN** 库中已存在用户 `alice` 时执行 `sebas auth add Alice`
- **THEN** 命令非零退出并报用户名已存在，提示改用 `sebas auth passwd`，用户库不变

#### Scenario: passwd 改密成功

- **WHEN** 库中存在用户 alice 时执行 `printf '%s' 'new-pw' | sebas auth passwd alice`
- **THEN** 该用户密码以新盐新哈希更新，输出确认信息

#### Scenario: passwd 用户不存在报错

- **WHEN** 库中不存在用户 ghost 时执行 `sebas auth passwd ghost`
- **THEN** 命令非零退出并报用户不存在，提示改用 `sebas auth add`，用户库不变

#### Scenario: list 输出账户清单

- **WHEN** 库中已有用户时执行 `sebas auth list`
- **THEN** 逐行输出用户名、角色、启用状态与时间戳，不含密码哈希

#### Scenario: list 零用户

- **WHEN** 用户库为零用户时执行 `sebas auth list`
- **THEN** 如实输出空列表，不编造条目

### Requirement: 缺省角色与显式角色

`add` 的缺省角色规则 SHALL 为：库零用户时建 **root**，否则建
**member**；`--role`（root/admin/member/viewer 四档）显式给出时覆盖
缺省。`passwd` SHALL NOT 接受 `--role`——角色调整归 WebUI root
管理面。

#### Scenario: 首户缺省 root

- **WHEN** 零用户库执行 `sebas auth add admin`
- **THEN** admin 以 root 角色创建

#### Scenario: 后续户缺省 member

- **WHEN** 库已有用户的情形下执行 `sebas auth add bob`
- **THEN** bob 以 member 角色创建

#### Scenario: --role 显式覆盖

- **WHEN** 执行 `sebas auth add vic --role viewer`
- **THEN** vic 以 viewer 角色创建，不取缺省规则

#### Scenario: 非法角色被拒

- **WHEN** 执行 `sebas auth add eve --role superadmin`
- **THEN** 命令非零退出并报角色非法，用户库不变

#### Scenario: passwd 拒绝 --role

- **WHEN** 执行 `sebas auth passwd alice --role admin`（alice 已存在）
- **THEN** 命令以参数错误非零退出，角色与密码均不变

### Requirement: 密码来源与校验

密码 SHALL 来自 `--password-stdin`（读一行，去尾部 CR/LF）或
`--password`，两者互斥；两者皆无或密码为空 SHALL 报错。少于 8 字符的
密码 SHALL 仅产生告警不拦截（CLI 面向测试环境与操作者自主权衡，与
首启设置页的 ≥8 硬门槛不同）。明文密码 SHALL 绝不落盘。

#### Scenario: stdin 读密

- **WHEN** 执行 `printf '%s' 'pw' | sebas auth add alice --password-stdin`
- **THEN** stdin 首行（去尾部 CR/LF）作为密码参与哈希，行尾换行不进入密码

#### Scenario: 来源互斥报错

- **WHEN** 同时给出 `--password` 与 `--password-stdin`
- **THEN** 命令以参数错误非零退出

#### Scenario: 缺密码报错

- **WHEN** `sebas auth add alice` 未给出任何密码来源
- **THEN** 命令非零退出并提示两种密码来源

#### Scenario: 空密码拒绝

- **WHEN** stdin 为空行时执行 `sebas auth passwd alice --password-stdin`
- **THEN** 命令非零退出并报密码不能为空，该用户密码不变

#### Scenario: 短密码告警不拦截

- **WHEN** 执行 `sebas auth add admin --password admin`（5 字符）
- **THEN** 创建成功并输出弱密码告警（stderr/日志），不因长度拒绝

#### Scenario: 明文不落盘

- **WHEN** 任一子命令以任意密码来源建户或改密
- **THEN** 用户库文件内容中检索不到该明文，只存在盐与哈希

### Requirement: 用户库路径与生效时机

用户库路径 SHALL 由 `SEBAS_WEBUI_AUTH_DB` 环境变量解析，缺省
`~/.sebas/auth.db`，与 webui 运行时同源；SHALL NOT 引入 config 键或
CLI 专属覆盖 flag。用户库即活数据：`add` 与 `passwd` 的变更 SHALL
即时生效，运行中的 webui 进程按请求实时读库。`[service.webui] auth`
开关关闭时 `auth` 命令 SHALL 仍可管理用户。

#### Scenario: env 覆盖路径

- **WHEN** 设置 `SEBAS_WEBUI_AUTH_DB=<沙箱路径>` 后执行 `sebas auth add admin`
- **THEN** 用户库创建在沙箱路径，真实 `~/.sebas/auth.db` 不被触碰

#### Scenario: 改密对运行中 webui 即时生效

- **WHEN** webui 运行中执行 `sebas auth passwd alice` 改密
- **THEN** alice 的既有会话在下次请求时按新密码校验生效，无需重启 webui

#### Scenario: 开关关闭仍可管理

- **WHEN** 配置 `service.webui.auth = false` 时执行 `sebas auth add admin`
- **THEN** 建户照常成功（为重新启用鉴权做准备），不产生任何强制登录效果
