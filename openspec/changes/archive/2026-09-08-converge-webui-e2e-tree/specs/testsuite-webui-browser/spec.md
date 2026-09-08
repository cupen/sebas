## Purpose

Defines the functional tree (capability → sub-capability → scenario) that the webui browser-e2e suite MUST follow, so that new cases grow under an existing sub-capability instead of branching off parallel coverage requirements.

## ADDED Requirements

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

## MODIFIED Requirements

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