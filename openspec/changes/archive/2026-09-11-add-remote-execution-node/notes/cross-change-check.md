# 跨 change 核对：`permission-flow` 是否被双写

> 任务：`tasks.md` 9.5。核对对象：已归档的
> `2026-09-10-unify-permission-approval-vocabulary`（下称「归档 change」）
> 与本 change `add-remote-execution-node`（下称「本 change」）。
> 结论先行：**无同一 requirement 的双写**。

## 方法

1. 逐条列出两侧 delta 的 requirement 名（`## MODIFIED Requirements` /
   `## ADDED Requirements` 下的 `### Requirement:` 标题）。
2. 与主 spec `openspec/specs/permission-flow/spec.md` 对基线，确认归档 change
   已落进主 spec、本 change 的 `MODIFIED` 基线文本与主 spec 逐字一致。
3. 扫描 `openspec/changes/*/specs/permission-flow/` 是否存在**第三个**未归档
   change 同时改这个 capability。

## 逐条比较

| | 归档 change（2026-09-10-unify-permission-approval-vocabulary） | 本 change（add-remote-execution-node） |
|---|---|---|
| 动作 | `MODIFIED Requirements` | `MODIFIED Requirements` + `ADDED Requirements` |
| Requirement 名 1 | **Three decision outcomes**（修改） | **Fail-closed on missing responder**（修改） |
| Requirement 名 2 | — | **Session mode gates whether a decision is requested**（新增） |
| Requirement 名 3 | — | **Remote approval requests survive control-plane absence**（新增） |

归档 change 的 `specs/permission-flow/spec.md` 只含一条 delta：
`### Requirement: Three decision outcomes`（正文重写为「三档决策 + cross-driver
词表归 `agent-driver` 管」）。

本 change 的 `specs/permission-flow/spec.md`：
- `MODIFIED`：`### Requirement: Fail-closed on missing responder`——在原文之后补
  「远程会话的 control plane 暂时不可达 SHALL NOT 算作无法应答；请求保持 parked；
  parked 期间无本地裁决路径」，并新增场景 `unreachable control plane parks
  instead of denying`；
- `ADDED`：`### Requirement: Session mode gates whether a decision is requested`；
- `ADDED`：`### Requirement: Remote approval requests survive control-plane absence`。

**两侧 requirement 名集合互不相交**：{Three decision outcomes} ∩
{Fail-closed on missing responder, Session mode gates whether a decision is
requested, Remote approval requests survive control-plane absence} = ∅。

## 基线一致性核验

- 主 spec `openspec/specs/permission-flow/spec.md` 的 `Three decision outcomes`
  正文（含三个场景）与归档 change 的 delta 文本一致 ⇒ 归档 change **已应用**，
  本 change 是在其产物之上继续写。
- 主 spec 的 `Fail-closed on missing responder` 正文（第 94 行）与本 change
  delta 里 `MODIFIED` 段落的基线文本（`... unknown request_id.`）逐字一致 ⇒
  本 change 改的是主 spec 当前实际内容，不是改归档 delta 的文本。

## 其他在飞 change

`grep` 结果：`openspec/changes/` 下只有本 change 存在
`specs/permission-flow/spec.md`。归档区中的 change 不再参与 delta 合成。因此不
存在第三个 writer。

## 判定

**无 double-write。** 推理：

1. 两个 change 修改的是同一 capability（`permission-flow`）下**不同的
   requirement**：归档 change 改 `Three decision outcomes`，本 change 改
   `Fail-closed on missing responder` 并新增两条。
2. 归档 change 的 delta 已应用进主 spec；本 change 的 `MODIFIED` 基线文本与主
   spec 一致，不会与归档产物争夺同一段 requirement 文本。
3. 无第三个未归档 change 改此 capability。

语义上两者相关但边界清楚：归档 change 统一的是「三档决策的词表/呈现」；本 change
不动该词表，只补充「远程会话在 control plane 缺席期间不算 fail-closed 场景」、
「mode 作为是否发请求的前置门」与「远程审批经链路往返并在缺席期间保活」。二者
不冲突，也不重复作者同一 requirement 文本。
