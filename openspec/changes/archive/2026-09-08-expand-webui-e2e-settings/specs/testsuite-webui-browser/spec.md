# testsuite-webui-browser Specification（三期：设置面与项目选择器）

## Purpose

在首批与二期旅程基础上，把设置面（Services/About/Env/agent-defaults/provider mutations）与项目添加对话框（folder-picker 树、内联错误）的浏览器级覆盖补齐到"只读呈现可信、写降级诚实"水平。本 spec 只定覆盖方向与验收口径，不锁具体 case 清单。

## ADDED Requirements

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
