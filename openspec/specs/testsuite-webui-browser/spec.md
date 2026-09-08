# testsuite-webui-browser Specification

## Purpose

在既有首批旅程基础上，把 webui 四大核心面（项目管理、会话管理、模型管理、agent 对话）的浏览器级覆盖补齐到"核心功能可用"水平。本 spec 只定覆盖方向与验收口径，不锁具体 case 清单；后续按本 spec 渐进补充用例，覆盖面只增不减。

## Requirements

### Requirement: 沙箱装配与清理边界

套件 SHALL 以一次性沙箱目录装配被测后端：`sebas core --router --debug --webui` 单进程形态，agent 为仓库自带 fake-claude 桩；MUST 覆盖全部默认路径与凭据 env（state DB、state file、provider overlay、core secret、auth file），MUST NOT 绑定 9797 或读写真实 `~/.sebas`。套件结束（无论成败）SHALL 结束后端进程并删除沙箱目录——POSIX 上后端 SHALL 优雅退出（SIGTERM），Windows 上允许硬终止、目录删除尽力而为；失败时 SHALL 保留现场目录并在输出中给出路径，供排障复用。

#### Scenario: 启动即隔离

- **WHEN** 套件拉起后端
- **THEN** 全部状态（config、DB、media、ACP 会话目录、auth 文件）位于一次性沙箱目录，webui 端口 ≠ 9797，进程 env 不含指向真实 `~/.sebas` 的路径

#### Scenario: 成功退出清理

- **WHEN** 全部用例通过、套件退出
- **THEN** 后端进程结束、沙箱目录被删除、端口恢复可用；POSIX 上后端为 SIGTERM 优雅退出，Windows 上允许硬终止（清理尽力而为）

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

套件 SHALL 在鉴权关闭与开启两种形态下分别验证：关闭时应用免登录直达 workbench；开启时未登录访问被重定向到登录页、错误凭据被拒绝、admin/admin 登录成功进入 workbench、登出后回到未鉴权态。沙箱装配 SHALL 支持以参数切换两种形态（统一测试账户 admin/admin）。

#### Scenario: 免登录直达

- **WHEN** 鉴权关闭时打开根路径
- **THEN** 直接渲染 workbench，无登录页

#### Scenario: 登录闭环

- **WHEN** 鉴权开启时以 admin/admin 登录、登出
- **THEN** 登录后进入 workbench，登出后再次访问需重新登录；错误凭据被拒绝且不进入应用

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
- **子功能 流式分批**：流式触发词触发分批 chunk 与运行中瞬态，无固定 sleep。
- **子功能 输入守卫**：空/空白不建回合；特殊字符与长文本能完整往返。

具体用例 SHALL 围绕上述子功能展开。本 requirement 不穷举轮数、文本种类与后端组合；后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: 首回合往返

- **WHEN** 操作员在 composer 输入文本提交
- **THEN** transcript 依次出现用户消息与桩的固定回复，会话状态收敛为 Done

#### Scenario: 同会话多轮连续

- **WHEN** 在同一会话内连续提交两轮文本
- **THEN** 两次回合均按序追加、双 Done、重载后内容不丢

#### Scenario: 流式分批渲染

- **WHEN** 以流式触发词发起回合
- **THEN** 内容分批出现且中途存在运行中的瞬态状态，最终回合完成

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

套件 SHALL 覆盖设置弹窗只读分区的浏览器呈现：Services 分区的 Router/路由状态卡片、About 分区的构建信息表、Env 分区的环境变量表。具体用例 SHALL 以对应 JSON API（`/api/router`、`/api/about`）为真值做包含断言（不断字面量：listen 地址、uptime 随沙箱而变），Env 分区 SHALL 断关键变量行存在且值为占位语义（不泄露真实值）。

#### Scenario: 只读分区与 API 对账

- **WHEN** 打开设置弹窗并切换到 Services/About/Env 分区
- **THEN** 各分区渲染值与 API 真值一致（listen/provider 数/version），无需修订本 spec 即可接纳新增分区用例

### Requirement: 设置面写操作诚实降级覆盖

沙箱无 control secret 时一切 router mutation 注定 503，套件 SHALL 覆盖该形态下的诚实语义：agent-defaults 置默认、provider 新建/编辑/删除/探测的失败 SHALL 以内联错误外显（`.callout-error`），provider 列表与 defaults 真值 SHALL 不变，对话框 SHALL 保持可交互（可取消重试）。客户端前置校验（如空名称）SHALL 不经过网络即报错。具体用例 SHALL 只断"失败外显与状态不变"，不断错误文案字面量；写持久化不断言（待 control-secret 沙箱形态）。

#### Scenario: 写降级失败外显且状态不变

- **WHEN** 在沙箱中执行任一设置面写操作
- **THEN** 内联错误可见、服务端列表与 defaults 与操作前一致，无需修订本 spec 即可接纳新增写面用例

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
