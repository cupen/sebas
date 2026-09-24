# testsuite-webui-browser Specification

## Purpose

在既有首批旅程基础上，把 webui 四大核心面（项目管理、会话管理、模型管理、agent 对话）的浏览器级覆盖补齐到"核心功能可用"水平。本 spec 只定覆盖方向与验收口径，不锁具体 case 清单；后续按本 spec 渐进补充用例，覆盖面只增不减。

## Requirements

### Requirement: 沙箱装配与清理边界

套件 SHALL 以一次性沙箱目录装配被测后端：`sebas core --config <path>
--webui` 与 `sebas router --config <path> --debug` **两进程形态**（router
不再内嵌于 core；core 旗标不含 `--router`），agent 为仓库自带 fake-claude
桩；MUST 覆盖全部默认路径与凭据 env（state DB、state file、provider
overlay、core secret、auth file），MUST NOT 绑定 9797 或读写真实
`~/.sebas`。套件结束（无论成败）SHALL 结束后端进程（含 router 子进程）
并删除沙箱目录——POSIX 上后端 SHALL 优雅退出（SIGTERM），Windows 上允许
硬终止、目录删除尽力而为；失败时 SHALL 保留现场目录并在输出中给出路径，
供排障复用。

#### Scenario: 启动即隔离

- **WHEN** 套件拉起后端
- **THEN** 全部状态（config、DB、media、ACP 会话目录、auth 文件）位于一次性沙箱目录，webui 端口 ≠ 9797，进程 env 不含指向真实 `~/.sebas` 的路径；core 与 router 为两个独立进程且 router 带 debug test provider

#### Scenario: 成功退出清理

- **WHEN** 全部用例通过、套件退出
- **THEN** 后端进程（core 与 router）结束、沙箱目录被删除、端口恢复可用；POSIX 上后端为 SIGTERM 优雅退出，Windows 上允许硬终止（清理尽力而为）

#### Scenario: 失败保留现场

- **WHEN** 任一用例失败
- **THEN** 套件报告失败并在输出中保留沙箱目录路径，目录内含后端日志可供复现

### Requirement: 会话核心旅程

套件 SHALL 在浏览器中覆盖会话核心旅程：从 workbench composer 发起会话（ACP 后端）→ 用户消息与桩回复按序出现在 transcript → 回合结束状态收敛为完成；流式回合 SHALL 可观测到内容分批到达与运行中的瞬态状态；页面重载后 transcript SHALL 从持久化状态恢复；桩触发"拒绝"时 SHALL 呈现非终态错误且会话存活（下一条消息仍可用），触发"崩溃"时 SHALL 如实呈现子进程死亡。

#### Scenario: 首回合往返

- **WHEN** 操作员在 composer 输入文本提交
- **THEN** transcript 依次出现用户消息与桩的固定回复，会话状态收敛为 Done

#### Scenario: 流式分批渲染

- **WHEN** 以流式触发词发起回合
- **THEN** 内容分批出现且中途存在运行中的瞬态状态，最终回合完成

#### Scenario: 重载恢复

- **WHEN** 完成一回合后刷新页面并回到该会话
- **THEN** transcript 与状态从持久化恢复，内容不丢

#### Scenario: 错误呈现

- **WHEN** 桩触发非终态拒绝与子进程崩溃两种错误
- **THEN** 前者在 UI 呈现错误且会话继续可用，后者如实呈现死亡而非伪装成功

### Requirement: 审批卡片旅程

套件 SHALL 覆盖 gated tool call 的浏览器审批闭环：桩发起需审批的工具调用时 review card SHALL 出现；allow-once 后工具结果以允许语义呈现；deny 后工具以拒绝语义呈现且回合结束；allow-session 后同一会话的同类后续工具调用不再弹卡。全部决策 SHALL 经真实审批 API 回传，fail-closed 语义保持（不决策则不执行）。

#### Scenario: 拒绝路径

- **WHEN** review card 出现后操作员点击 deny
- **THEN** 卡片消失，transcript 呈现拒绝语义的工具结果，回合完成

#### Scenario: 单次允许路径

- **WHEN** 操作员点击 allow-once
- **THEN** 工具结果以允许语义呈现，回合完成

#### Scenario: 会话级允许

- **WHEN** 操作员点击 allow-session 后再次触发同类工具调用
- **THEN** 不再出现 review card，工具直接执行

### Requirement: 鉴权与访问旅程

套件 SHALL 在鉴权关闭、鉴权开启已有用户、鉴权开启零用户三种形态下分别验证：关闭时应用免登录直达 workbench 且无任何门禁元素（含可达性轮询窗口内）；开启已有用户时未登录访问被重定向到登录页、错误凭据被拒绝、admin/admin 登录成功进入 workbench、登出后回到未鉴权态；开启零用户时渲染首启设置页（root 建户就地校验、建户进入 workbench、会话经刷新存活、再次 setup 请求被 409 拒绝），且该页 SHALL 在可达性轮询窗口内保持稳定不被翻转成登录页。沙箱装配 SHALL 支持以参数切换三种形态（登录形态统一测试账户 admin/admin；零用户形态不预建户）。

#### Scenario: 免登录直达

- **WHEN** 鉴权关闭时打开根路径
- **THEN** 直接渲染 workbench，无登录页

#### Scenario: 登录闭环

- **WHEN** 鉴权开启时以 admin/admin 登录、登出
- **THEN** 登录后进入 workbench，登出后再次访问需重新登录；错误凭据被拒绝且不进入应用

#### Scenario: 首启 root 引导

- **WHEN** 鉴权开启且用户库零用户时打开根路径（第三种沙箱形态）
- **THEN** 渲染首启设置页并跨越两个可达性轮询间隔保持稳定（不翻转成登录页）；建 root 后进入 workbench 且会话经刷新存活；再次 setup 请求被 409 拒绝

### Requirement: 工作台、项目与会话管理面旅程

套件 SHALL 覆盖：workbench 首屏结构（项目栏、composer、summary/reachability 如实显示沙箱真实状态）；项目添加与移除在项目栏即时生效；会话 close 后从活动列表消失；archive 后从列表隐藏且归档视图可见该条目；会话深链在 SPA fallback 下可直达；退役路径（如 /settings）canonical 重定向到 `/`；无模型选项的 ACP 会话不显示模型下拉且不报错（D4 诚实缺省）。

#### Scenario: 首屏与 reachability

- **WHEN** 打开根路径
- **THEN** 项目栏、composer、summary 呈现，reachability 显示沙箱内各执行体的真实状态

#### Scenario: 项目增删

- **WHEN** 添加一个沙箱内目录为项目再将其移除
- **THEN** 项目栏先后呈现与移除该项目

#### Scenario: close 与 archive

- **WHEN** 对已有会话先后执行 close 与 archive
- **THEN** close 后会话离开活动列表；archive 后归档视图可见该条目、活动列表不再显示

#### Scenario: 深链与退役路径

- **WHEN** 直接访问会话深链与退役路径
- **THEN** 深链经 SPA fallback 渲染对应会话，退役路径重定向到 `/`

#### Scenario: 模型面诚实缺省

- **WHEN** 打开无模型选项的桩会话
- **THEN** 模型下拉不渲染、无控制台错误，模型切换请求得到如实的拒绝语义

### Requirement: 一键入口与稳定性

`invoke testsuite-webui` SHALL 是套件唯一入口：构建（含 dist 自动重建）→ 装配沙箱 → 运行全部用例 → 清理；支持按名过滤单个旅程；后续补充用例 SHALL 经既有过滤机制运行、无需新增入口。断言 SHALL 基于状态轮询而非固定 sleep（web-first + 轮询，禁固定 sleep）；同一提交重复运行 SHALL 稳定收敛（多次全绿为收尾门槛），清理与保留现场行为一致。**新增**：spec 文件 SHALL 维持两层 `test.describe` 结构——顶层对应大功能（与 requirement 一致）、二层对应子功能（与 COVERAGE 一致）；`invoke testsuite-webui` 入口 preflight SHALL 拒绝任何在 spec 文件顶层直接出现 `test(...)` 而未包在 `test.describe` 内的写法，避免新加用例时绕开功能树收敛。

#### Scenario: 一键运行

- **WHEN** 在仓库根执行 `invoke testsuite-webui`
- **THEN** 无需人工步骤完成构建、运行与清理，退出码反映通过与否

#### Scenario: 单旅程过滤

- **WHEN** 以旅程名过滤执行
- **THEN** 仅运行匹配的 spec 文件，清理行为与全量运行一致

#### Scenario: 渐进补用例不改入口

- **WHEN** 按覆盖方向 requirement 在现有子功能下补充新用例
- **THEN** 无需新增入口命令，仅经既有过滤机制运行，清理与保留现场行为一致；新用例必须在两层 `test.describe` 内

#### Scenario: 顶层裸 test 拒绝

- **WHEN** `invoke testsuite-webui` 入口 preflight 检查 spec 文件
- **THEN** 任何 spec 文件在顶层直接出现 `test(...)`（未包在 `test.describe` 内）则套件拒绝运行并报错指向该文件

### Requirement: 项目管理覆盖

套件 SHALL 覆盖项目栏的核心管理功能：项目增删改查的闭环、异常拒绝（非法路径、重复注册）、列表呈现与排序持久化。功能下 SHALL 划分以下子功能：

- **子功能 增删**：添加沙箱内目录为项目再移除，项目栏呈现与移除一致。
- **子功能 异常拒绝**：非法路径与重复注册在 400/409 内联错误呈现，注册表不变。
- **子功能 排序与持久化**：项目排序在重载后保持；git/非 git 项目分支呈现一致。
- **子功能 选择器交互**：folder-picker 树展开点选回填；空路径禁用提交；非法路径框不关、错误内显。

具体用例 SHALL 围绕上述子功能展开，单个用例只取其中一个切面。本 requirement 不穷举 case；后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: 项目增删

- **WHEN** 添加一个沙箱内目录为项目再将其移除
- **THEN** 项目栏先后呈现与移除该项目

#### Scenario: 项目异常拒绝

- **WHEN** 提交非法路径或重复注册既有项目
- **THEN** 服务端 400/409 返回、UI 内联错误外显、注册表未变

#### Scenario: 项目排序与分支呈现

- **WHEN** 调整项目排序后刷新页面；浏览 git 项目与非 git 项目
- **THEN** 排序持久化保持；分支标签按仓库真实状态如实呈现

#### Scenario: folder-picker 树展开点选回填

- **WHEN** 在 folder-picker 对话框展开父目录、点选子目录
- **THEN** 路径回填手动框、提交后项目落栏

#### Scenario: 选择器空路径与非法路径

- **WHEN** 手动框为空提交；提交一个不存在的路径
- **THEN** 前者按钮禁用；后者内联错误、对话框不关、注册表不变

### Requirement: 会话管理覆盖

套件 SHALL 覆盖会话生命周期的可逆操作与列表/路由语义：会话创建、切换/聚焦、close、archive/restore、归档态写保护。功能下 SHALL 划分以下子功能：

- **子功能 close 与 archive**：close 后会话离开活动列表；archive 后归档视图可见、活动列表隐藏。
- **子功能 深链与退役路径**：会话深链在 SPA fallback 下可直达；退役路径 canonical 重定向到 `/`。
- **子功能 多会话切换**：列表点击与深链直达两种方式在双会话间互切不串。
- **子功能 archive 写保护**：archive 后写操作得 400，restore 不复活会话（详情页如实 404）。

具体用例 SHALL 围绕上述子功能展开。本 requirement 不穷举 case；后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: close 与 archive

- **WHEN** 对已有会话先后执行 close 与 archive
- **THEN** close 后会话离开活动列表；archive 后归档视图可见该条目、活动列表不再显示

#### Scenario: 深链与退役路径

- **WHEN** 直接访问会话深链与退役路径
- **THEN** 深链经 SPA fallback 渲染对应会话，退役路径重定向到 `/`

#### Scenario: 多会话切换

- **WHEN** 在活动列表点击切换、并以深链直达另一会话
- **THEN** 两次切换均聚焦到目标会话，transcript 与状态互不串

#### Scenario: archive 写保护与 restore 诚实语义

- **WHEN** 对归档会话发起写操作、再访问其详情页
- **THEN** 写操作得 400；restore 不复活会话，详情页如实 404

### Requirement: 模型管理覆盖

套件 SHALL 覆盖模型面的呈现与切换语义：有模型选项会话的呈现与切换语义、无模型会话的诚实缺省、settings 内 provider 列表的只读呈现。功能下 SHALL 划分以下子功能：

- **子功能 无模型诚实缺省**：无模型会话不显示模型下拉、无控制台错误，模型切换请求得到如实的拒绝语义。
- **子功能 settings provider 只读**：settings 内 provider 列表只读呈现，不暴露写操作。

具体用例 SHALL 围绕上述子功能展开；不做真实模型拨测；harness 具备"有模型选项"会话是本 requirement 的前置（正向切换待驱动模型面立项）。本 requirement 不穷举 case；后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: 无模型会话 set_model 诚实拒绝

- **WHEN** 在无模型会话上发起模型切换请求
- **THEN** 切换以终态 Error 形式被拒、UI 如实呈现、详情页可继续浏览

#### Scenario: settings provider 只读

- **WHEN** 浏览 settings 内 provider 列表分区
- **THEN** 列表只读呈现、不暴露新建/编辑/删除入口；写操作如被发送则得 503 诚实外显

### Requirement: agent 对话覆盖

套件 SHALL 覆盖 agent 对话的核心功能：单会话内多轮消息往返、回合状态收敛、transcript 持久化恢复、composer 输入守卫。功能下 SHALL 划分以下子功能：

- **子功能 首回合往返**：从 composer 提交文本，transcript 依次出现用户消息与桩回复，回合收敛为 Done。
- **子功能 多轮连续**：同会话两轮连续问答，按序追加、双 Done、重载不丢。
- **子功能 重载恢复**：完成回合后刷新页面回到该会话，transcript 与状态从持久化恢复，内容不丢。
- **子功能 流式分批**：桩按时间间隔发出文本 delta；用例在会话仍处于运行态时于 DOM 观察到已上屏的增量正文，随后回合收敛为 Done；零固定 sleep、不依赖 retry 兜底。
- **子功能 输入守卫**：空/空白不建回合；特殊字符与长文本能完整往返。

为支撑「流式分批」，套件 SHALL 具备确定性构造「回合进行中」窗口的桩能力：桩按可配置的时间间隔逐段发出文本 delta，且该间隔落在 driver 的挂起（hang）探测预算内，使中途窗口既不因过快而不可观测、也不因超时被判为挂起。该能力 SHALL NOT 依赖真实模型凭据。

具体用例 SHALL 围绕上述子功能展开。本 requirement 不穷举轮数、文本种类与后端组合；后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: 首回合往返

- **WHEN** 操作员在 composer 输入文本提交
- **THEN** transcript 依次出现用户消息与桩的固定回复，会话状态收敛为 Done

#### Scenario: 同会话多轮连续

- **WHEN** 在同一会话内连续提交两轮文本
- **THEN** 两次回合均按序追加、双 Done、重载后内容不丢

#### Scenario: 流式分批渲染

- **WHEN** 以流式触发词发起回合，桩按时间间隔逐段发出文本 delta
- **THEN** 在回合到达终态之前，focused conversation 的 DOM 已出现增量正文（会话此时仍处于运行态）
- **AND** 该断言不依赖重试兜底；回合随后收敛为 Done

#### Scenario: composer 输入守卫

- **WHEN** 提交空字符串/纯空白；提交含特殊字符与长文本
- **THEN** 前者不建回合；后者完整往返

### Requirement: 项目管理核心功能覆盖

套件 SHALL 覆盖项目栏的核心管理功能：项目增删改查的闭环、异常拒绝（非法路径、重复注册）、列表呈现与排序持久化、与 folder-picker 选择器交互。本 requirement 下的子功能由 `项目管理覆盖` requirement 列出的「增删 / 异常拒绝 / 排序与持久化 / 选择器交互」承担；本 requirement 仅保留覆盖面渐进增长入口。具体用例 SHALL 围绕上述子功能展开，单个用例只取其中一个切面；后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: 项目管理覆盖面渐进增长

- **WHEN** 按 `项目管理覆盖` 的现有子功能补充新用例
- **THEN** 用例经浏览器驱动真实沙箱后端，断言项目栏状态与持久化一致，无需修订本 spec 即可接纳

### Requirement: 会话管理核心功能覆盖

套件 SHALL 覆盖会话生命周期的可逆操作与列表/路由语义。本 requirement 下的子功能由 `会话管理覆盖` requirement 列出的「close 与 archive / 深链与退役路径 / 多会话切换 / archive 写保护」承担；本 requirement 仅保留覆盖面渐进增长入口。具体用例 SHALL 围绕上述子功能展开；后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: 会话管理覆盖面渐进增长

- **WHEN** 按 `会话管理覆盖` 的现有子功能补充新用例
- **THEN** 用例经浏览器驱动真实沙箱后端，断言活动列表/归档视图/焦点路由一致，无需修订本 spec 即可接纳

### Requirement: 模型管理核心功能覆盖

套件 SHALL 覆盖模型面的呈现与切换语义。本 requirement 下的子功能由 `模型管理覆盖` requirement 列出的「无模型诚实缺省 / settings provider 只读」承担；本 requirement 仅保留覆盖面渐进增长入口。具体用例 SHALL 围绕上述子功能展开；不做真实模型拨测；harness 具备"有模型选项"会话是本 requirement 的前置（见 design D1）。后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: 模型管理覆盖面渐进增长

- **WHEN** 按 `模型管理覆盖` 的现有子功能补充新用例
- **THEN** 用例经浏览器驱动真实沙箱后端，断言模型呈现与切换语义诚实，无需修订本 spec 即可接纳

### Requirement: agent 对话核心功能覆盖

套件 SHALL 覆盖 agent 对话的核心功能：单会话内多轮消息往返、回合状态收敛、transcript 持久化恢复、composer 输入守卫。本 requirement 下的子功能由 `agent 对话覆盖` requirement 列出的「首回合往返 / 多轮连续 / 重载恢复 / 流式分批 / 输入守卫」承担；本 requirement 仅保留覆盖面渐进增长入口。具体用例 SHALL 围绕上述子功能展开；后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: 对话覆盖面渐进增长

- **WHEN** 按 `agent 对话覆盖` 的现有子功能补充新用例
- **THEN** 用例经浏览器驱动真实沙箱后端，断言 transcript 顺序、状态收敛与重载恢复一致，无需修订本 spec 即可接纳

### Requirement: 设置面只读呈现覆盖

套件 SHALL 覆盖设置弹窗只读分区的浏览器呈现：Services 分区的 Router 服务
状态卡片、About 分区的构建信息表、Env 分区的环境变量表。具体用例 SHALL 以
对应 JSON API（Services 分区以 `/api/admin/services`、About 分区以
`/api/about`）为真值做包含断言（不断字面量：listen 地址、uptime 随沙箱而
变），Env 分区 SHALL 断关键变量行存在且值为占位语义（不泄露真实值）。退役
的 `GET /api/router` SHALL 不再作为任何分区的真值来源。

#### Scenario: 只读分区与 API 对账

- **WHEN** 打开设置弹窗并切换到 Services/About/Env 分区
- **THEN** 各分区渲染值与 API 真值一致（服务 desired/actual/uptime、
  provider 数/version），无需修订本 spec 即可接纳新增分区用例

### Requirement: 设置面写操作诚实降级覆盖

core 不可达时一切 provider 写操作注定 503，套件 SHALL 覆盖该形态下的诚实
语义：provider 新建/编辑/删除/探测与 defaults 写入的失败 SHALL 以内联错
误外显（`.callout-error`），provider 列表与 defaults 真值 SHALL 不变，对
话框 SHALL 保持可交互（可取消重试）。客户端前置校验（如空名称）SHALL 不
经过网络即报错。具体用例 SHALL 只断「失败外显与状态不变」，不断错误文案
字面量；写持久化不断言（待 core 可达沙箱形态）。

#### Scenario: 写降级失败外显且状态不变

- **WHEN** 在 core 不可达沙箱中执行任一设置面写操作
- **THEN** 内联错误可见、服务端列表与 defaults 与操作前一致，无需修订本
  spec 即可接纳新增写面用例

### Requirement: 项目选择器交互覆盖

套件 SHALL 覆盖添加项目对话框的两条非 happy-path 交互：folder-picker 树 SHALL 支持懒加载展开（父目录展开后子目录出现）与点选回填（选中后手动路径框同步、提交后项目落栏）；非法输入 SHALL 内联报错且对话框不关闭（空路径时提交按钮禁用；不存在路径提交后错误外显、注册表不变）。树懒加载触及的 Web Awesome 已知渲染异常（`nextSibling`）仍按既有过滤口径容忍。

#### Scenario: 树选与非法输入闭环

- **WHEN** 经树点选添加项目，或提交空/非法路径
- **THEN** 树选路径下项目正常落栏；非法输入下错误内显、对话框不关、注册表不变

### Requirement: detached 部署态旅程

套件 SHALL 提供 detached 双进程沙箱变体（core + 独立 webui，auth 关闭），供部署态旅程在浏览器级验证——现有浏览器沙箱为 `core --webui` 单进程形态，通道客户端不在场，无法触达"核心不可达"路径。

#### Scenario: 一键运行 detached 旅程用例

- **WHEN** `invoke testsuite-webui --case deployment` 以 detached 变体运行
- **THEN** chromium 指向独立 webui 端口，旅程按下列场景执行并在结束自清理

### Requirement: 核心不可达横幅与降级提示旅程

在 detached 变体下，套件 SHALL 验证全局横幅与降级提示的诚实呈现。

#### Scenario: core 停止后横幅出现

- **WHEN** 双进程装配达可达后终止 core，页面处于任意视图
- **THEN** 全局"核心不可达"横幅出现且包含上报 cause

#### Scenario: core 停止期间加项目出现降级提示

- **WHEN** core 停止期间经项目选择器注册一个合法目录
- **THEN** 项目落栏且伴随"核心不可达，已写入本地注册表"降级提示；composer 提交门禁同时呈现不可达态

#### Scenario: core 恢复后横幅与降级态消失

- **WHEN** core 重新启动并完成通道握手
- **THEN** 无需刷新页面，横幅消失，composer 恢复可提交

### Requirement: ACP 桩驱动的浏览器呈现覆盖

浏览器级工作台旅程中凡断言 **ACP 驱动器行为**（并行审批卡片、thinking 呈现、流式结算形态）的用例，SHALL 以 `fake-claude` 桩剧本经浏览器沙箱装配驱动；沙箱配置按既有惯例（claude-empty / claude-stream）为所需剧本各提供独立 agent 装配段。

**与 `extend-test-model-scenarios` 的双载体分工（有意并行，非重复覆盖）**：同一浏览器呈现面在两条执行通路上各有独立用例，两者的**事件生产者不同**——本 change 经 ACP 子进程（`fake-claude` 桩）产生审批请求与流式帧，`extend-test-model-scenarios` 经 router 内置 `test` 模型（native 内核通路）产生。浏览器呈现层虽同，但从生产者到 UI 的链路（ACP 驱动解析 / hook 泊车 / 帧投递 vs native 内核直投）不同，任一通路的回归都不能被另一通路发现。因此 `并行审批卡片` 在两侧各有一条浏览器旅程：本侧为桩驱动的 ACP 通路权威，test 模型侧为 native 通路权威；两侧 SHALL NOT 相互替代、SHALL NOT 因对方存在而豁免，账本按各自 capability 分别记行。桩剧本无法驱动而只能由 router test 模型驱动的浏览器呈现，SHALL 留给 `extend-test-model-scenarios`。

#### Scenario: 并行审批卡片各自独立（桩驱动）

- **WHEN** 以并行剧本 agent 会话在浏览器提交一个多工具任务
- **THEN** 两个审批卡片各自独立出现（独立卡片、独立工具名/参数），而非一张合并卡
- **AND** 逐一决策后回合继续推进，两个工具结果如实呈现

#### Scenario: thinking 过程折叠在浏览器中真链路呈现

- **WHEN** 以 thinking 剧本 agent 会话在浏览器完成一个 thinking/正文交替的回合
- **THEN** thinking 段落以过程折叠（collapsed process fold）呈现，正文独立于折叠之外按序展示
- **AND** 回合结算后 thinking 折叠与正文顺序保持、内容不丢失

#### Scenario: 流式正文在回合结算时切换为 markdown 渲染

- **WHEN** 以流式触发词（drip）会话在浏览器提交消息，turn_engaged 期间观察增量正文
- **THEN** 回合进行中增量正文以 live-tail 纯文本形态上屏（与 conversation-streaming 既有断言一致）
- **AND** 回合结算后同一正文转为 markdown 渲染且内容拼接一致，无重复或丢段

### Requirement: test 场景驱动的浏览器呈现覆盖

浏览器级工作台旅程中凡断言 **LLM 响应形状**的用例，SHALL 以 `test/<scenario>` 场景会话驱动（凡会话执行路径可达 router 的形态——含 native 内核经 router URL 的通路；native 通路的可用性遵循 `testsuite-acceptance`「native 链路验收策略」的 spike 门控）。`fake-claude` 桩的既有用例继续作为其**驱动器专属契约**（deny / crash / 流式触发词等 ACP 驱动行为）的权威，迁移按账本节奏进行且 SHALL NOT 降低覆盖口径。

**与 `add-acp-stream-approval-journeys` 的双载体分工（有意并行，非重复覆盖）**：`并行审批卡片` 的浏览器旅程在两侧各有一条，因两者的**事件生产者与执行通路不同**——本侧经 router 内置 `test` 模型（native 内核通路）产生审批请求与流式帧，该 change 经 ACP 子进程（`fake-claude` 桩）产生。浏览器呈现层虽同，生产者到 UI 的链路（native 内核直投 vs ACP 驱动解析 / hook 泊车 / 帧投递）不同，任一通路的回归都不能被另一通路发现。两侧 SHALL NOT 相互替代、SHALL NOT 因对方存在而豁免，账本按各自 capability 分别记行。同理，`流式中经 UI 取消` 亦为双载体：既有 `stop-settle.spec.ts` 以 `fake-claude --slow-ms` 桩驱动（ACP 通路，账本 ✅），本 spec 的 `test/long` 用例是 native 通路的对应件，不构成重复。

test 模型使其**可确定性驱动、且桩驱动不了**的浏览器呈现 SHALL 纳入覆盖方向（新增用例按本 spec 既有规则落子功能并在 `COVERAGE.md` 加行）：

- 同回合多个 tool_use 的**并行审批卡片**（各自独立弹出与决策）；
- 零输出回合的**通知呈现**；
- **流式中经 UI 取消**（流停止、取消如实呈现、会话可继续）；
- **UI 模型切换后下一回合行为变化**（切换端到端生效的可见证明）。

#### Scenario: 并行审批卡片在浏览器中各自独立

- **WHEN** 以 `test/tools-parallel` 会话在浏览器提交一个多工具任务
- **THEN** 每个工具调用的审批卡片各自独立出现
- **AND** 逐一决策后回合继续推进

#### Scenario: 零输出通知在浏览器中呈现

- **WHEN** 以 `test/empty` 会话提交一条消息
- **THEN** 回合完成且 transcript 无可见输出
- **AND** 零输出通知出现在会话面

#### Scenario: 流式中经 UI 取消

- **WHEN** 以 `test/long` 会话流式呈现期间经 UI 发起取消
- **THEN** 流停止、取消如实呈现
- **AND** 会话此后可发起新回合

#### Scenario: UI 模型切换下一回合生效

- **WHEN** 会话先以 `test/text` 完成一回合，经 UI 切换模型为 `test/tool-use` 后提交含工具的任务
- **THEN** 下一回合出现审批卡片（新场景行为生效）
- **AND** 切换前后的回合各自保留其呈现形状
