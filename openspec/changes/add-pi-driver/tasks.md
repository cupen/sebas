# add-pi-driver — Tasks

## 1. 协议层（sebas-acp/src/pi/）

- [x] 1.1 建 `sebas-acp/src/pi/codec.rs`：pi RPC 帧的 serde 类型（命令/响应/会话事件三族，`pi.id` 关联）与严格 LF 分帧读写器（容忍前导 CR、字节流切分不依赖通用行读取器），附 U+2028/U+2029 出现在 JSON 字符串内的分帧单测（红：错切；绿：正确分帧）
- [x] 1.2 实现 `translate_event`：pi 会话事件 → `AcpEvent` 映射（text/thinking delta、tool_execution_*、usage、agent_settled/abort、进程退出终态），以录制帧 fixture 离线单测锁形状（对齐 ipc golden fixture 惯例），验证见 `cargo test -p sebas-acp pi::codec`
- [x] 1.3 实现 `translate_command`：`AcpCommand` → pi RPC 命令（send→prompt、SetModel→set_model、取消→abort、握手期 get_state/get_available_models），含 `pi.id` 关联与 `set_model` 失败响应的翻译单测

## 2. PiDriver 实现

- [x] 2.1 `sebas-acp/src/pi/driver.rs`：`AgentDriver::spawn` 实现——argv 组装（`--mode rpc`、`--session-dir`、恢复时 `--session <id>`）、子进程 spawn、stdin/stdout 双泵、stderr 只记日志；用假二进制（脚本回放 fixture 帧）单测验证握手四元组上报（routing id/resumed/pi 会话 id/模型面）
- [x] 2.2 事件循环与命令回路：事件翻译进 `evt_tx`、命令消费、`agent_settled` 后才发 `Finished`、取消 = abort + 等 settled、进程退出 → terminal Error；startup timeout 强制执行（超时杀进程并按既有口径失败），假二进制挂起场景单测验证超时生效
- [x] 2.3 恢复语义：`--session <id>` 重挂与被拒诚实回落新会话（resumed=false + 告警）；单测用假二进制分别模拟「id 可挂接」与「id 被拒」两个应答形状
- [x] 2.4 `sebas-acp/src/lib.rs` 导出新模块与 driver 类型，`cargo test -p sebas-acp` 全绿

## 3. 配置与装配

- [x] 3.1 `src/config.rs`：`AgentConfig::Pi(AcpPiConfig)`（path 默认 `pi`、sessions_dir、work_dir、args 键值形式、两超时，镜像 claude 形态）+ `command()/driver_tag()/display()/work_dir()/startup_timeout()/idle_kill_secs()` 补全；config 解析单测（合法/未知键拒绝/args 位置参数报错口径与 claude 一致）
- [x] 3.2 装配点：`run.rs::build_agent_registry` 与 `agent_store.rs::ensure_registered` 两处 match 加 `Pi → PiDriver`；`sebas-models` `is_valid_driver`/`AgentDefinition::command`、`sebas-dispatch` `validate_agent_definition` 放行 `pi` 标签；`cargo test -p sebas-models -p sebas-dispatch` + 相关单测绿
- [x] 3.3 reachability：二进制在场探测复用既有路径，`pi auth check`（ready/not_ready/invalid）作补充信号；`agent_kinds` 探测单测（假 pi 二进制分别回 ready/not_ready）验证目录如实区分

## 4. 默认 agent 切换

- [x] 4.1 `AcpConfig` 回退链：零 agent 时探测 `pi` → `claude`（命中写启动日志，皆缺按既有口径失败）；`default_kind()/default_kind_binary()` 单测覆盖：显式赢、单 agent 隐式、零 agent 探测链两分支
- [x] 4.2 `config/config.toml.example`：种子 `[acp.agents.pi] driver = "pi"` + 保留 claude 条目 + 显式 `default = "pi"`，注释说明切换方法；配置解析集成测试断言示例文件可加载且默认解析为 pi
- [x] 4.3 诚实退化：默认 agent 二进制缺失时目录报不可达 + 未指定 agent 的新会话失败并点名缺失（不静默改选）；webui catalog 层单测验证 cause 透传

## 5. WebUI

- [x] 5.1 后端：`/api/agents` 目录与 spawn 解析对 pi 行零特殊分支（driver 不上 wire 契约不变），`pnpm --filter webui test`（或既有前端测试入口）里补 pi 行目录快照断言
- [x] 5.2 前端 `settings-modal.ts`：driver 形态选择加 `pi`（二进制路径默认 `pi` + sessions 目录字段），封闭标签集 `claude|acp|pi`；组件测试验证 pi 形态建行 → 存储标签 `pi` → 目录立即可选

## 6. 集成与验收

- [x] 6.1 进程级 e2e journey（`tests/testsuite_e2e_test.rs` 风格，skip-if-absent：pi 二进制不在场跳过并在输出如实声明）：沙箱钉 `PI_CODING_AGENT_SESSION_DIR`/HOME，pi 侧 `models.json` 指向 `sebas fake-provider`，验证创建会话 → 回合完成 → 重启恢复 → 切模型 → 取消四条旅程
- [x] 6.2 watchdog 探针兼容验证：pi 活会话上周期 `set_permission_mode` 得到非终态「不支持」、会话存活不受影响，纳入 6.1 journey 断言
- [x] 6.3 沙箱手工验收配方（AGENTS.md debug recipe 补一节 pi 段落）：fake-provider 上游 + pi 自定义 provider 的完整配置样例；按配方手跑一轮并记录结论
- [x] 6.4 `openspec validate add-pi-driver --strict` 通过；全量 `cargo test` + 前端测试 + `invoke testsuite-e2e`（pi 缺席时 journey 如实跳过）无回归
