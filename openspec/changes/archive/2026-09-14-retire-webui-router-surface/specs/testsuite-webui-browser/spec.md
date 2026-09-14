## MODIFIED Requirements

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
