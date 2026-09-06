## Context

extract-im-service 已落地(`sebas im` 子命令、`ServiceName::Im`、`[watchdog.im]`)并归档,但其 delta 是改名前撰写的——归档时已就地修正了 watchdog delta 的 10 处 gateway 措辞,仍有三处子命令/进程面未跟上(见 proposal)。全部为 spec 对齐,零行为变化。

## Goals / Non-Goals

**Goals:**

- 子命令树、控制面服务列表、场景措辞、glossary 与 im-service 落地后的现实一致。

**Non-Goals:**

- 不改任何代码;不动现行别名与 SHALL NOT 拒绝条款;不重写刚归档的能力。

## Decisions

- **纯 MODIFIED(场景标题不变)**:三处需求都只改需求正文或场景 THEN 措辞,场景标题全部保留,因此普通 MODIFIED 全文拷贝即可通过校验,无需 fix-spec-gateway-residue 用过的 early-sync 模式。
- **glossary 直接改**:glossary.md 不是 spec,不参与 delta 机制,随 apply 直接编辑 run 词条。

## Risks / Trade-offs

- [MODIFIED 块漏拷场景导致归档时丢内容] → 三处需求均已逐字比对当前主 spec(webui 主控部署形态 3 个场景、Control request surface 3 个场景、Subcommand tree 3 个场景)。

## Migration Plan

纯文档变更,归档即生效。
