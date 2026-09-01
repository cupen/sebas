# Tasks: add-architecture-doc

## 1. 文档骨架与进程全景

- [x] 1.1 创建 `docs/architecture.md`,写入文档定位与约定(贡献者视角、2026-08 实现快照、关键结论标注来源文件的格式约定)。验证:文件存在,开头一段说明清晰交代定位与维护方式。
- [x] 1.2 撰写进程全景章节:单一 `sebas` 二进制、子命令决定进程角色(watchdog / gateway / webui / control CLI / record / replay / update / service),附 ASCII 全景图。验证:图中每个子命令都能与 `src/main.rs` 的 `Cmd` 枚举分支一一对应。

## 2. 核心章节

- [x] 2.1 撰写 watchdog 监督模型章节:core / webui / gateway 三服务职责边界、spawn → readiness 门 → 崩溃退避的生命周期、默认启动策略(仅 WebUI 默认启用,core/gateway 默认停用并经服务页持久化到 services.json)。核对 `src/watchdog/supervisor.rs`、`src/watchdog/services.rs`、`src/config.rs`。验证:章节含生命周期状态图,表述与上述文件中的代码注释一致。
- [x] 2.2 撰写 IPC 语义章节:管道仅承载 readiness 握手(`{"cmd":"ready"}`),控制操作(StartCore / StopCore / RestartCore / Update / Rollback / ServiceSet / Status)走 Unix socket control RPC,WebUI 服务页与 CLI 共用同一通道。核对 `src/ipc.rs`、`src/watchdog/control_rpc.rs`、`src/watchdog/control.rs`。验证:文档中的 RPC 操作列表与 `ControlRequest` 枚举成员完全一致。
- [x] 2.3 撰写 CLI 控制面与 crate 职责速查:`sebas ctl/status/services` 的用途、根 crate 与 5 个成员 crate(sebas-router / sebas-feishu / sebas-acp / sebas-gateway / sebas-webui)各自一句话职责。核对 `Cargo.toml` members 与各 crate `lib.rs` 文档注释。验证:crate 列表与 `Cargo.toml` 完全一致,无遗漏无多余。
- [x] 2.4 为关键架构结论补充来源标注(文件路径 + 模块名),覆盖进程拓扑、IPC、默认策略三类结论。验证:随机抽查 5 处标注,`grep`/`ls` 确认所引文件真实存在且内容相符。

## 3. 收尾

- [x] 3.1 通读校对全文:确认覆盖 `proposal.md` What Changes 承诺的全部内容,未混入未来计划或未实现的设想。验证:对照 proposal.md 逐项勾对,并在 change 目录留一句校对结论。
- [x] 3.2 在 `README.md` 新增且仅新增一行指向 `docs/architecture.md` 的链接(不改写任何既有内容)。验证:链接路径正确,`git diff README.md` 仅显示新增的一行。
