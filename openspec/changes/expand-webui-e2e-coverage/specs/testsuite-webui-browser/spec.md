# testsuite-webui-browser Specification（增量二期）

## Purpose

在既有首批旅程基础上，把 webui 四大核心面（项目管理、会话管理、模型管理、agent 对话）的浏览器级覆盖补齐到"核心功能可用"水平。本 spec 只定覆盖方向与验收口径，不锁具体 case 清单；后续按本 spec 渐进补充用例，覆盖面只增不减。

## ADDED Requirements

### Requirement: 项目管理核心功能覆盖

套件 SHALL 覆盖项目栏的核心管理功能：项目的添加与移除、列表呈现、以及管理动作的持久化语义。具体用例 SHALL 围绕"增删改查的闭环与异常拒绝"展开，单个用例只取其中一个切面；本 requirement 不穷举 case，后续补充用例 SHALL 落在本 requirement 下、无需改 spec。

#### Scenario: 项目管理覆盖面渐进增长

- **WHEN** 按本 requirement 补充新的项目管理用例
- **THEN** 用例经浏览器驱动真实沙箱后端，断言项目栏状态与持久化一致，无需修订本 spec 即可接纳

### Requirement: 会话管理核心功能覆盖

套件 SHALL 覆盖会话列表的核心管理功能：会话的创建、切换/聚焦、close、archive/restore 往返，以及归档态的写保护语义。具体用例 SHALL 围绕"会话生命周期的可逆操作"展开；本 requirement 不穷举 case，后续补充用例 SHALL 落在本 requirement 下、无需改 spec。

#### Scenario: 会话管理覆盖面渐进增长

- **WHEN** 按本 requirement 补充新的会话管理用例
- **THEN** 用例经浏览器驱动真实沙箱后端，断言活动列表/归档视图/焦点路由一致，无需修订本 spec 即可接纳

### Requirement: 模型管理核心功能覆盖

套件 SHALL 覆盖模型面的核心管理功能：有模型选项会话的模型呈现与切换语义、无模型会话的诚实缺省、settings 内 provider 列表的只读呈现。具体用例 SHALL 只断"切换语义与呈现诚实"，不做真实模型拨测；harness 具备"有模型选项"会话是本 requirement 的前置（见 design D1）。本 requirement 不穷举 case，后续补充用例 SHALL 落在本 requirement 下、无需改 spec。

#### Scenario: 模型管理覆盖面渐进增长

- **WHEN** 按本 requirement 补充新的模型管理用例
- **THEN** 用例经浏览器驱动真实沙箱后端，断言模型呈现与切换语义诚实，无需修订本 spec 即可接纳

### Requirement: agent 对话核心功能覆盖

套件 SHALL 覆盖 agent 对话的核心功能：单会话内消息往返、回合状态收敛、transcript 持久化恢复，以及 composer 输入守卫。具体用例 SHALL 围绕"多轮可用与输入鲁棒"展开；本 requirement 不穷举轮数、文本种类与后端组合，后续补充用例 SHALL 落在本 requirement 下、无需改 spec。

#### Scenario: 对话覆盖面渐进增长

- **WHEN** 按本 requirement 补充新的对话用例
- **THEN** 用例经浏览器驱动真实沙箱后端，断言 transcript 顺序、状态收敛与重载恢复一致，无需修订本 spec 即可接纳

### Requirement: 一键入口与稳定性

`invoke webui-e2e` SHALL 保持唯一入口语义并容纳新增用例：支持按名过滤，断言遵循既有纪律（web-first + 轮询，禁固定 sleep），同一提交多次全绿为收尾门槛。

#### Scenario: 渐进补用例不改入口

- **WHEN** 按上述任一 requirement 补充新用例
- **THEN** 无需新增入口命令，仅经既有过滤机制运行，清理与保留现场行为一致
