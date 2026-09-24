# testsuite-process-e2e Specification

## Purpose

以真实二进制组成 detached 拓扑（独立 core 进程 + 独立 `sebas webui` 进程经核心通道相连）对核心流程做进程级端到端校验，全部路径沙箱化，并提供一键运行入口，让"核心流程是否正确"可以单条命令验证。

## Requirements

### Requirement: 一键运行入口

套件 SHALL 提供单条命令入口 `invoke testsuite-e2e`：先构建工作区二进制（含 `sebas` 与 `fake-claude`），再运行全部进程级 e2e 用例。套件 MUST 也能不经 invoke 直接以 `cargo test --test testsuite_e2e_test -- --ignored` 运行。命令的退出码 SHALL 如实反映套件通过与否。

#### Scenario: 一条命令完成构建与校验

- **WHEN** 操作员在仓库根执行 `invoke testsuite-e2e`
- **THEN** 工作区完成构建后套件全部用例被执行，任一用例失败则命令以非零码退出，全部通过则以零码退出

#### Scenario: 不依赖 invoke 也可运行

- **WHEN** 操作员执行 `cargo test --test testsuite_e2e_test -- --ignored`
- **THEN** 套件用例同样全部运行（二进制已构建的前提下）

### Requirement: 沙箱全隔离

每个用例 MUST 运行在一次性沙箱内：配置文件落在 scratch 目录（含 dispatch state_file、media download_dir、acp sessions_dir/work_dir、service.core channel_path、service.webui host/port），端口 SHALL 不同于 9797；环境变量 MUST 全量覆盖默认值——伪造 `SEBAS_CORE_SECRET`，并显式设置 `SEBAS_STATE_DB`、`SEBAS_STATE_FILE`、`SEBAS_ROUTER_PROVIDER_OVERLAY` 指向沙箱路径。套件 MUST NOT 读写操作员真实 `~/.sebas`、真实凭据或占用其端口。

#### Scenario: 与操作员实例完全隔离

- **WHEN** 套件在存有运行中操作员实例（端口 9797、真实 `~/.sebas`）的机器上执行
- **THEN** 所有进程只绑定沙箱端口、只读写沙箱目录，操作员实例不受任何影响

#### Scenario: 用例结束清理沙箱

- **WHEN** 任一用例结束（无论通过或失败）
- **THEN** 其 scratch 目录被清理（保留给事后排查的除外），不遗留守护进程

### Requirement: 单用例手动运行与现场保留

套件用例 SHALL 使用语义化且稳定的名称，使单个用例可经 cargo 过滤器手动运行：`cargo test --test testsuite_e2e_test <用例名> -- --ignored`；`invoke testsuite-e2e` SHALL 提供 `--case` 参数把用例名透传为该过滤器（缺省仍运行全部）。任一用例失败时 MUST 保留其沙箱目录（含核心与 webui 日志）并向输出打印路径，供事后排查；保留目录落在 `target/tests/` 下，由 `cargo clean` 兜底清理。

#### Scenario: 按名称单独运行一个用例

- **WHEN** 开发者执行 `invoke testsuite-e2e --case <用例名>` 或等价的 cargo 过滤命令
- **THEN** 仅该用例被运行，其余用例不执行，退出码如实反映该用例结果

#### Scenario: 失败保留现场

- **WHEN** 任一用例失败
- **THEN** 该用例的沙箱目录（含 core 与 webui 日志）被保留，输出打印沙箱与日志路径，可按同一配置手动复现

### Requirement: detached 拓扑启动可达性

套件 SHALL 验证 detached 形态启动：核心 `sebas core`（router debug 模式）与独立 `sebas webui` 进程经核心通道连接后，webui 的 `/health` 返回 ok，`/api/summary` 的 `reachability.ok` 为 true。

#### Scenario: 双进程启动后 webui 报告可达

- **WHEN** 核心 webui 两个进程按沙箱配置启动且核心通道握手完成
- **THEN** webui `GET /health` 返回 ok，`GET /api/summary` 返回 `reachability.ok = true`

### Requirement: 会话往返全链路

套件 SHALL 经 webui 进程的 HTTP 面验证完整会话往返：创建会话（ACP 执行体）→ 消息经核心通道驱动 ACP 子进程（fake-claude 桩）→ 会话状态到达 Done 且应答内容可查。

#### Scenario: 创建会话到回合完成

- **WHEN** `POST /api/sessions` 以 ACP 后端提交一条文本，随后轮询会话状态
- **THEN** 会话最终状态为 Done，会话详情含 fake-claude 的应答文本

### Requirement: router debug provider 应答

套件 SHALL 验证核心内置 debug 路由可用：对路由 `/v1/messages` 以 debug `test` 模型发起请求返回 200 与固定应答。

#### Scenario: test 模型请求命中内置应答

- **WHEN** 向沙箱路由 `POST /v1/messages` 提交 `model = "test"` 的请求
- **THEN** 返回 200 且应答为 debug provider 的固定内容

### Requirement: 错误核心密钥拒连

套件 SHALL 验证密钥不匹配时连接被如实拒绝：以与核心不同的 `SEBAS_CORE_SECRET` 启动 webui，webui 不得以"已连接"状态继续服务。

#### Scenario: 密钥不匹配时可达性如实上报失败

- **WHEN** webui 进程携带错误 `SEBAS_CORE_SECRET` 启动并请求 `/api/summary`
- **THEN** `reachability.ok` 为 false 且给出 cause，不出现虚假的已连接状态

### Requirement: 核心重启期间可达性翻转

套件 SHALL 验证核心生命周期变化在 webui 侧如实可见：核心进程退出后 `reachability.ok` 翻转为 false（含 cause），核心重新启动后翻回 true。

#### Scenario: 核心停止再拉起

- **WHEN** 核心进程被终止，随后以同一沙箱配置重新启动
- **THEN** 停止窗口内 webui `/api/summary` 的 `reachability.ok` 为 false，核心恢复后变回 true

### Requirement: 优雅退出清理

在支持 SIGTERM 的平台上（unix 门控），套件 SHALL 验证核心收到 SIGTERM 后优雅退出：核心通道 socket 文件被移除，会话状态落盘。

#### Scenario: SIGTERM 后通道痕迹消除

- **WHEN** 向运行中的核心发送 SIGTERM 并等待其退出
- **THEN** channel_path 的 socket 文件不再存在，状态文件包含退出前会话状态

### Requirement: 有界时间与平台门控

每个用例 MUST 以显式超时为界(禁止无界等待),套件总时长 SHOULD 控制在数分钟内。套件 MUST 在 Linux 与 Windows 双平台通过编译并可运行——不得存在未门控的平台性编译期依赖;平台相关用例(如 SIGTERM)MUST 按平台条件编译门控,在不支持的平台上跳过且不判失败。套件用例 MUST 以 `#[ignore]` 标注,不进入默认 `cargo test` 路径。

#### Scenario: 平台不支持时跳过而非失败

- **WHEN** 套件在无 SIGTERM 语义的平台(如 Windows)上运行
- **THEN** 平台门控用例被跳过,其余用例正常运行,套件整体不因此失败

#### Scenario: 挂起子进程不拖垮套件

- **WHEN** 任一外部依赖(核心/webui/ACP 子进程)未在用例超时内给出预期响应
- **THEN** 该用例在超时处失败并留下可诊断的日志/残留路径,套件其余用例继续执行

#### Scenario: 双平台编译通过

- **WHEN** 在 Linux 与 Windows 上分别执行 `cargo test --test testsuite_e2e_test --no-run`
- **THEN** 两平台均编译成功,不存在因平台差异引发的编译错误

### Requirement: 无密钥装配旅程（事故回归）

套件 SHALL 验证不注入 `SEBAS_CORE_SECRET` 的 detached 双进程装配：core 与独立 webui 均无 env secret，core 靠自动武装、webui 靠 secret 文件发现完成连接。此为真机"socket absent"事故的回归用例。

#### Scenario: 双进程均无 secret 时启动即可达

- **WHEN** core 与独立 webui 按沙箱配置启动且两者环境均无 `SEBAS_CORE_SECRET`
- **THEN** webui `/api/summary` 的 `reachability.ok` 变为 true，secret 文件存在于解析路径
- **AND** 经 webui HTTP 面完成一次会话往返（创建 → Done）

#### Scenario: 既有 env 注入路径不回归

- **WHEN** 既有带 `SEBAS_CORE_SECRET` 的沙箱用例照常运行
- **THEN** 全部保持绿（env 优先语义未破坏既有装配）

### Requirement: 密钥轮换自愈旅程

套件 SHALL 验证 core 重启换钥后，不重启的 webui 自动恢复：杀掉 core → 重启 core（新随机 secret 覆写 secret 文件）→ webui 在重连退避内恢复可达。

#### Scenario: 重启 core 后 webui 不重启自愈

- **WHEN** 双进程装配达 reachable 后 core 被终止并以同 config 重启
- **THEN** webui 进程不重启，期间 reachability cause 如实呈现，随后 `reachability.ok` 恢复 true

### Requirement: 监督重启恢复旅程

套件 SHALL 以 watchdog 监督形态验证崩溃自愈（收窄验收账本缺口 #3）：watchdog 拉起 core + webui，杀掉 core，supervisor 按重启策略自动拉起，webui 随之恢复。

#### Scenario: watchdog 自动重启被杀的 core

- **WHEN** watchdog 监督下的 core 进程被杀死
- **THEN** supervisor 在重启延迟内重新拉起 core，通道 socket 重新出现，webui `reachability.ok` 恢复 true

### Requirement: mode 透传与中途切换用例

进程级 e2e 套件 SHALL 覆盖 agent mode 链路，全部用例走既有桩（fake-claude 等），零真模型调用：

- **argv 透传用例**：以带/不带 `mode` 各创建一个本机 claude 会话，经 fake-claude journal 断言子进程 argv 是否含映射后的 `--permission-mode` 值（ask/缺省 → 无该参数，edit → acceptEdits，allow/auto → bypassPermissions）。
- **行为差异化用例**：以 mode=allow 创建会话并发送 `perm` 场景消息，断言门控放行、回合完成且未产生审批卡片（与 ask 模式下同场景产生审批形成对照）。
- **中途切换用例**：对运行中的 fake-claude 会话 POST mode 切换，断言 journal 记录运行时模式切换、快照 mode 更新（接受路径），以及失败路径产生非致命错误且会话存活。
- **拒绝用例**：未知 mode 的创建/切换请求返回 400。

#### Scenario: argv 透传断言

- **WHEN** 以 mode=allow 创建会话并完成一个回合
- **THEN** journal 中该子进程 argv 含 `--permission-mode bypassPermissions`；不带 mode 的对照会话 argv 不含该参数

#### Scenario: allow 模式下 perm 场景免审批

- **WHEN** 以 mode=allow 创建的会话收到 `perm` 场景消息
- **THEN** fake-claude 不产生 PreToolUse 审批交互，工具直接执行并完成回合

#### Scenario: 中途切换记录于 journal

- **WHEN** 对运行中的 fake-claude 会话切换 mode
- **THEN** journal 记录该运行时权限模式切换，会话快照的 mode 更新为新值

### Requirement: Windows 进程树收割

沙箱拆卸在 Windows 上 MUST 收割被记录子进程的整棵进程树(含核心派生的孙进程,如 ACP 子进程与被监督重生的核心),不留孤儿进程占用沙箱端口或锁住沙箱目录;对已退出目标的收割尝试 SHALL 被视为成功,不阻塞沙箱目录的清理。

#### Scenario: Windows 拆卸不留孤儿

- **WHEN** 任一用例在 Windows 上结束(无论通过或失败),且核心在用例期间派生过子进程
- **THEN** 沙箱拆卸后不存在仍占用沙箱端口或持有沙箱目录句柄的孤儿进程,沙箱目录可被删除

### Requirement: harness 平台安全

一键入口与沙箱清理流程 MUST 在 Windows 与 Linux 上均安全执行:进程存活探测 SHALL 在任何平台上都不得终止被探测进程;清理流程 SHALL 仅作用于确认为陈旧沙箱的目标;不得因引用目标平台不存在的信号而中断入口流程。

#### Scenario: Windows 存活探测不误杀

- **WHEN** 在 Windows 上执行 `invoke testsuite-e2e` 触发陈旧沙箱清理,探测到存活进程的 pid
- **THEN** 探测动作不终止该进程,仅陈旧沙箱记录的进程被清理

#### Scenario: 入口不因平台缺失的信号中断

- **WHEN** 在 Windows 上执行 testsuite 入口及其拆卸路径
- **THEN** 全程不因引用平台缺失的信号(如 SIGKILL)抛异常而中断

### Requirement: provider 透传 journey

套件 SHALL spawn fake 上游并在沙箱 config 声明自定义 provider（`base_url_anthropic` 指向 fake、上游 key 为哑值），经路由 `/v1/messages` 验证进程级透传链路：上游 key 注入且下游 key 不泄漏（fake journal 离线断言）、SSE 逐事件透传、usage 结算落 router-usage.jsonl。

#### Scenario: 非流式透传与 usage 落账

- **WHEN** 向 router 发起 model 路由到 fake provider 的非流式请求
- **THEN** 应答为 fake 的确定性内容，router-usage.jsonl 新增记录含非零 input/output tokens 与该 provider 名

#### Scenario: 流式透传完整

- **WHEN** 发起 stream=true 的同类请求
- **THEN** 客户端收到完整 SSE 事件序列，usage 结算同样落账

#### Scenario: 下游 key 不泄漏到上游

- **WHEN** 检查 fake 的请求 journal
- **THEN** 转发请求的 auth header 为 provider 上游 key，且不含下游 key 与 hop-by-hop 头

### Requirement: agent-loop journey（零 token）

在 claude-code 二进制可用的环境，套件 SHALL 以真实 claude-code 为 ACP 执行体、会话模型路由到 fake 上游，验证完整 agent 工具环：消息 → tool_use → 工具执行 → tool_result → 终文本，会话到达 Done；二进制缺席时该用例 SHALL 跳过而非失败。

#### Scenario: 工具环到 Done

- **WHEN** 创建模型路由到 fake 的 ACP 会话并提交一条触发工具的任务
- **THEN** 会话状态到达 Done，回合内容含终文本与工具执行痕迹，全程无真实上游外呼

#### Scenario: claude-code 缺席诚实跳过

- **WHEN** 测试环境不存在 claude-code 二进制
- **THEN** 该用例跳过并输出原因，套件整体不判失败

### Requirement: 确定性限流/用量用例

套件 SHALL 以 fake 上游验证 per-key 令牌桶限流与用量记录的确定性：fake 秒回应答消除真实上游网络延迟抖动，使限流断言可精确复现，且全程不向真实上游发起网络请求；usage 计量断言使用 fake 的确定性非零 usage。

#### Scenario: 令牌桶耗尽 429 可复现

- **WHEN** 以超过桶容量的速率向 router 连发请求（fake 秒回）
- **THEN** 越界请求收到 429 rate_limit_error，且该用例重复运行结果一致、无网络外呼

### Requirement: ACP 桩并行工具环剧本

`fake-claude` 桩 SHALL 提供「并行工具环」驱动剧本（触发词形态，与既有 perm / tool-loop 剧本同机制）：单个 ACP 回合产生两个 tool_use，并连发两个 hook_callback 审批请求使其**同时待批**，两个请求都在待批中时才开始等待决定；每个请求收到决定后各自落 tool_result（allow → 成功文本，deny → is_error），全部落定后输出环后正文并以 result 正常收尾。既有场景与触发词的行为 SHALL NOT 改变。

#### Scenario: 单回合并发产生两个待批请求

- **WHEN** 以触发词 `parallel` 驱动桩完成一个回合
- **THEN** 该回合先后发出两个不同 request_id 的 hook_callback 审批请求，且第二个发出时第一个仍待批
- **AND** 两个请求逐一收到决定后，回合以终文本与 result 正常收尾

#### Scenario: 决定组合逐请求生效

- **WHEN** 两个待批请求收到不同决定组合（allow/allow、allow/deny、deny/deny）
- **THEN** 每个 tool_use 的 tool_result 与其自身收到的决定一致，互不影响
- **AND** deny 不阻止另一工具的执行与回合收尾

### Requirement: ACP 并行权限进程级旅程

验收套件 SHALL 含一条经 webui 用户面（HTTP API + WS）驱动的进程级 journey：以并行剧本会话提交一回合，断言两个审批请求各自独立泊车与决策、回合在全部决策后推进至终态，且全程无真实上游外呼。

#### Scenario: 并行权限环 journey 全绿

- **WHEN** 进程级 e2e 套件以并行剧本会话执行「提交 → 两请求泊车 → 逐一决策 → 回合推进」旅程
- **THEN** journey 断言两个 request_id 各自独立出现、决策按请求生效、会话终态正确
- **AND** 重复执行旅程断言转录形状与终态一致（id/时间戳除外）

### Requirement: 旅程证明标准——绿即工作台可用且稳定

test 模型 journey 的价值在于其**证明力**，套件 SHALL 按以下标准设计与断言，使 journey 全绿即构成「agent 工作台正常、稳定、可用」的证据：

- **用户面保真**：journey SHALL 经工作台的用户面驱动——webui HTTP API 提交操作、WS/SSE 接收事件——不得为图省事绕过用户面直调内部接口；断言的对象 SHALL 是操作员可见的行为（转录呈现、会话状态、通知），不是内部数据结构。
- **路径完整**：journey 集 SHALL 合计覆盖操作员完整路径——项目注册/选择 → 创建会话 → 提交消息 → 流式呈现 → 权限决策 → 结果确认 → 关闭/归档；任一环节无 journey 命中即为缺口。
- **可重复即稳定**：场景的确定性 SHALL 被 journey 消费——同一 journey 重复执行 SHALL 产生相同的转录形状与会话终态，套件 SHALL 至少对一条代表性 journey 断言重复一致性；单次绿不构成稳定性证明。
- **失败也诚实**：错误路径 journey SHALL 同时断言「失败如实呈现」与「失败后工作台仍可操作」（其余会话可继续创建与使用）。

#### Scenario: 完整操作员路径在用户面走通

- **WHEN** 以 webui HTTP API 注册项目、创建 native 会话（模型 `test/tool-use`）、经 WS 提交消息并完成一次权限批准
- **THEN** 流式事件、权限请求、终文本均出现在用户面
- **AND** 会话经用户面关闭，全程零真实上游外呼

#### Scenario: 重复执行结果一致

- **WHEN** 代表性 journey（工具环）连续执行两次（独立会话）
- **THEN** 两次的转录块形状、权限交互次数与会话终态一致
- **AND** 差异仅允许出现在时间戳与 id 类字段

#### Scenario: 会话取消在流式中生效

- **WHEN** 以 `test/long` 驱动长流会话，流式期间经用户面发起取消
- **THEN** 流式停止、取消被如实呈现（非伪装完成）
- **AND** 会话此后仍可发起新回合或被关闭

#### Scenario: 模型切换在下一回合生效

- **WHEN** 会话先以 `test/text` 完成一回合，再经用户面把模型切换为 `test/tool-use` 并提交含工具的任务
- **THEN** 下一回合的 LLM 行为符合新场景（产生 tool_use 与权限请求）
- **AND** 切换前后的回合各自保留其呈现形状

#### Scenario: 失败后工作台仍可操作

- **WHEN** 一个会话因 `test/error` 回合失败后
- **THEN** 操作员仍可创建新会话并完成一次正常回合（如 `test/text`）
- **AND** 工作台不因单个会话的失败进入不可用状态

### Requirement: test 模型场景 journey

套件 SHALL 以 debug `test` provider 的场景模型驱动进程级验收 journey，覆盖核心五簇中**LLM 形状相关**的工作台能力，全程零真实凭据、零上游外呼、零额外进程（router 内自答，区别于 fake 上游的拨号 journey）。场景与被覆盖能力的对应关系 SHALL 按 `extend-test-model-scenarios` 的覆盖矩阵执行：

- **native kernel 工具环 journey**（`test/tool-use`）：tool_use → 权限请求 → 批准 → 工具执行 → tool_result → 终文本 → Done；
- **并行权限 journey**（`test/tools-parallel`）：一回合多个 tool_use 各自产生独立权限请求，逐一批准/拒绝后回合继续；
- **thinking 呈现 journey**（`test/thinking`）：thinking 进转录且与正文呈现形态可区分；
- **混排 journey**（`test/full`）：thinking / 正文 / tool_use 同回合按序呈现；
- **零输出通知 journey**（`test/empty`）：回合完成但无可见输出时追加通知；
- **长文流式 journey**（`test/long`）：流式背压路径与增量呈现，拼接结果与非流式一致；
- **错误呈现 journey**（`test/error`）：LLM 失败如实呈现（错误进回合/会话状态），不伪装成功；
- 可选：真实 claude-code 场景 journey（缺席诚实跳过，与 agent-loop journey 同法）。

对话连续性 SHALL 由 echo 句式显式证明：`test/text` 的应答回显最后一条用户消息，journey SHALL 断言该回显内容与会话历史一致，使「完整对话历史送达 LLM」成为被断言的事实而非隐含假设。

各 journey 的断言 SHALL 使用场景的确定性形状（块顺序、stop_reason、固定非零 usage），且既有的 bare `test` echo 断言 SHALL 不改动而通过。

#### Scenario: native kernel 工具环经权限流到 Done

- **WHEN** 以 native 后端创建模型为 `test/tool-use` 的会话并提交一条任务
- **THEN** 首回合产生 tool_use 并触发权限请求，批准后工具执行、tool_result 回传
- **AND** 次回合返回终文本，会话到达 Done，usage 记录为场景的确定性非零值，全程无真实上游外呼

#### Scenario: 并行工具调用各自获得独立权限请求

- **WHEN** 以 `test/tools-parallel` 驱动一个声明了多个工具的会话
- **THEN** 同一回合的每个 tool_use 各自产生独立的权限请求（互不串扰）
- **AND** 逐一决策后回合继续推进

#### Scenario: thinking 进入转录且与正文可区分

- **WHEN** 创建模型为 `test/thinking` 的会话并提交一条任务
- **THEN** 回合转录含 thinking 内容块，其呈现形态与正文块可区分
- **AND** 会话正常到达 Done

#### Scenario: echo 回显证明对话历史完整送达

- **WHEN** 会话经历多轮消息后以 `test/text` 完成一回合
- **THEN** 应答中回显的正是本轮最后一条用户消息的原文
- **AND** 更早轮次的消息不污染回显（回显取的是最后一条）

#### Scenario: 零输出回合追加通知

- **WHEN** 创建模型为 `test/empty` 的会话并提交一条任务
- **THEN** 回合完成且转录无可见输出
- **AND** 通知路径被触发（零输出通知出现在会话面）

#### Scenario: 长文流式保持拼接一致

- **WHEN** 创建模型为 `test/long` 的会话并提交一条任务
- **THEN** 流式呈现期间无帧丢失，最终拼接文本与非流式 body 一致
- **AND** 会话正常到达 Done

#### Scenario: LLM 错误如实呈现

- **WHEN** 创建模型为 `test/error` 的会话并提交一条任务
- **THEN** 失败以错误状态如实呈现（回合/会话面可见错误，不伪装成功）
- **AND** 会话状态迁移遵循既有终局错误语义

#### Scenario: 既有 echo 断言不改而通过

- **WHEN** 套件中既有的 bare `test` debug 应答用例运行
- **THEN** 断言不加改动而通过
- **AND** 场景模型的引入未改变 bare `test` 的响应
