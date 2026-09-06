## Context

rename-cli-surface 归档后,主 specs 大面积换用 router 措辞,但逐文件扫描发现 4 个能力仍有少量段落把 "gateway" 当现行术语使用(见 proposal)。经与代码对照(`sebas-dispatch/src/engine/provider_card.rs` 按钮文案、`sebas-webui/src/server.rs` 路由、`src/config.rs` 配置节)确认这些是漏改而非兼容语义。此外改名遗留了一层兼容实现(旧 env 名回退、配置节迁移 shim、状态值 serde alias、隐藏 CLI 别名),而项目从未正式发布,不存在需要兼容的存量部署。

## Goals / Non-Goals

**Goals:**

- 主 specs 中作为现行术语的 "gateway" 全部改为 "router",与 glossary、代码、其余 spec 段落一致。
- 拆除全部 rename 兼容层:代码与 spec 同步,旧 env 名/旧配置节/旧状态值/旧 CLI 别名一律不再被接受。

**Non-Goals:**

- 不改现行命名:`[watchdog.*]` 配置节、watchdog 守护进程概念、`status`/`services`/`ctl` 现行别名。
- 不动 webui 退役路径 `/gateway` 重定向(现行设计)。
- 不改其余能力 spec 的行为语义。

## Decisions

- **兼容层直接删除而非保留窗口**:项目未发布,无存量用户/配置/systemd unit;"一个窗口内自动迁移并告警"的机制(rename-cli-surface 引入)整体移除,而不是再等一个窗口。旧值处理策略选**响亮失败**(如状态值 `"kind": "gateway"` 解析报错点名文件),不静默猜测。
- **MODIFIED 全文拷贝 + early-sync 同步主 spec**:delta 的 MODIFIED 块携带目标全文(含改名的场景标题、替换后的拒绝场景)。openspec v1.12.0 无法经 delta 表达场景改名(校验按场景标题严格匹配,场景级 RENAMED 不被解析;REMOVED+ADDED 同名视为冲突),而 apply 对"delta 与主 spec 已一致"的 MODIFIED 走 early-sync 短路——因此实施时先把主 specs 直接改到与 delta 完全一致(tasks 组 1),validate 随即通过,archive 识别为已同步、干净跳过。
- **cli-service 的兼容条款改为显式 SHALL NOT**:子命令树与 env 变量需求不仅删掉兼容句,还明确旧名 SHALL NOT 被接受并各给一个拒绝场景,作为拆除工作的可测验收。
- **保留 pre-rename 提及中描述现行行为的部分**:如 webui 退役路径 `/gateway`(IA-v1 退役是现行设计);provider-management 的 legacy 状态值场景则改为"拒绝"语义而非保留。

## Risks / Trade-offs

- [MODIFIED 块漏拷场景导致归档时丢内容] → delta 与主 spec 同步后用 `openspec validate --strict` 复核(两者一致时该检查天然通过)。
- [删除兼容后测试/脚本里残留旧 env 名引用] → tasks 含全量 grep 复核(`SEBAS_GATEWAY`、`SEBAS_AGENT_GATEWAY`、`alias = "gateway"`、`watchdog.gateway` 等),COVERAGE.md 与 AGENTS.md 引用随查随改。

## Migration Plan

纯文档 + 兼容层拆除,无数据迁移(未发布,无存量状态文件/配置)。归档(rename→sync)即生效。
