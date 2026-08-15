# 飞书话题（thread/topic）多会话 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 sebas 在飞书话题（topic/thread）场景下把「每个话题 = 一个独立会话」真正跑通：话题内所有出站消息（每轮 root 卡、权限卡、初始卡、失败提示卡）回复到话题根消息，保证回复聚合在原话题；话题失效时提示用户而不是重发或静默失败。

**Architecture:** 会话隔离的骨架已存在（`SessionKey{chat_id, thread_id}`、事件已解析 `thread_id`、`SessionMap` 按 key 隔离）。本计划补三块：
1. **入站归一化**：`feishu/events.rs` 解析 `message.root_id`，话题内消息的 reply target 归一化为话题根消息 `message_id`（主线保持触发消息 `message_id`）。
2. **回复目标存储**：router 新增 per-SessionKey 的 `ReplyTargetMap`（纯内存），每次入站文本写入，权限卡出站时作为 `root_id` 带上。
3. **出站兜底**：初始 root 卡与失败提示卡（spawn/resume/session-lost）在话题 key 下也用该 reply target；飞书返回话题失效错误码（230019 话题不存在 / 230071 群不支持话题回复）时，向会话发一条文本提示，**不重试、不重发**。

**Tech Stack:** Rust（sebas crate = 2024，router/feishu = 2021），tokio，serde_json，reqwest，anyhow。不新增依赖。

## Global Constraints（grill-me 已与用户确认）

- **会话边界（Q1）**：群聊 + p2p 单聊里，每个话题 = 一个独立会话；话题外主线（`thread_id=None`）也是一个独立会话。代码零改动（`SessionKey` 已天然区分）。
- **回复聚合（Q2）**：话题内回复目标 = 话题根消息 `root_id`；主线保持现状（触发消息 `message_id`）。`reply_in_thread` 参数待云端实测后再定，本计划不引入。
- **触发方式（Q3）**：群聊维持 @ 触发，不申请全量消息敏感权限；p2p 天然免 @。
- **最小范围（Q6）**：事件解析补 `root_id`、出站回复目标、权限卡跟随、单测；**不动** `/new`、`/sessions`（未实现）、不加配置开关。
- **话题失效（Q8）**：230019/230071 → 发普通文本提示「该话题已失效…」，不重试、不重发。
- **分支与工作区**：基于 main 开 `feat/feishu-thread-sessions`，在 `.worktree/feat/feishu-thread-sessions` 下开发；测试验收后推送远端，用户云端实测后走 rebase + merge --no-ff。

## File Structure

| 文件 | 责任 | 动作 |
|---|---|---|
| `feishu/src/events.rs` | 事件解析 | 解析 `message.root_id`；`FeishuIn::Text`/`Media` 的 reply_to 归一化（话题=root_id，主线=message_id）；`Media` 增加 `reply_to` 字段 |
| `feishu/src/client.rs` | 出站 API | 新增 `FeishuApiError{code,msg}` 类型化错误（`post_card_with_retry` 最终失败返回它）+ `is_topic_invalid()`；新增 `send_text` |
| `router/src/router/maps.rs` | per-key 存储 | 新增 `ReplyTargetMap`（`set`/`get`/`clear`） |
| `router/src/router/mod.rs` | RouterHandle | 加 `reply_targets` 字段 + 公开 `reply_target()` getter |
| `router/src/router/inbound.rs` | 入站路由 | `dispatch` 的 Media 分支透传 reply_to；`on_text` 顶部写入 reply target |
| `router/src/router/acp_events.rs` | 权限卡出站 | 权限卡 `SendCard` 的 root_id 在话题 key 下取 reply target |
| `src/session_boot.rs` | 初始 root 卡 | 初始卡 root_id 话题填充 + 失效提示兜底 |
| `src/dispatch.rs` | 出站分发 | spawn/resume 失败卡、session-lost 卡话题填充 + 失效提示兜底；共享 helper |
| `.gitignore` | 工作区 | 加 `.worktree/` |
| `docs/superpowers/plans/2026-08-09-feishu-thread-sessions.md` | 本计划 | 新建 |

## 关键语义（实现前钉死）

- 事件体字段：`im.message.receive_v1` 的 `message.root_id` = 话题根消息的 `message_id`（话题内消息都是"回复根消息"）；话题根消息本身无 `root_id` 但有 `thread_id`。归一化规则：`thread_id.is_some()` 时 reply target = `root_id.unwrap_or(message_id)`；否则 = `message_id`。
- `ReplyTargetMap` 在入站文本/媒体时更新：`Media` 也带归一化后的 `reply_to`，与 `Text` 一样经 `on_text` 写入 map。
- 权限卡 root_id：`key.thread_id.is_some()` 时从 `reply_targets` 取；主线保持 `None`（现状）。
- 出站兜底 helper（`src/dispatch.rs`，`pub(crate)`）：`topic_reply_target(router, key, root_id)` 只在 `root_id` 为空且 `key.thread_id.is_some()` 时填充；`send_card_with_topic_fallback` 捕获 `FeishuApiError`，命中 `is_topic_invalid()` 则发文本提示并返回空 message_id（不冒泡错误）。
- 飞书卡片消息体 30KB 上限等既有约束不变。

## 测试矩阵

| 层 | 用例 | 断言 |
|---|---|---|
| feishu events | 话题内子消息（thread_id+root_id） | key.thread_id=Some；reply_to=root_id |
| feishu events | 话题根消息（thread_id、无 root_id） | reply_to=message_id |
| feishu events | 主线消息（无 thread_id） | reply_to=message_id（现状不变） |
| feishu events | 话题内媒体消息 | Media 带 reply_to=root_id |
| router | 话题内文本 → 权限请求 | SendCard.root_id=话题根消息 |
| router | 主线文本 → 权限请求 | SendCard.root_id=None（现状不变） |
| feishu client | FeishuApiError downcast + is_topic_invalid | 230019/230071 命中，其他不命中 |

## 云端实测清单（用户执行）

- [ ] 群聊话题形式群：开两个话题，各自独立对话，回复聚合在原话题
- [ ] 群聊话题：权限卡出现在话题内、点击可正常 allow/deny
- [ ] p2p 单聊：对机器人消息做话题回复，话题内对话独立、免 @ 续聊
- [ ] 失效场景：删除/失效一个话题后发消息 → 收到「话题已失效」提示，不重发

## 已知限制（本计划不做）

- 不申请全量群消息权限：话题内每条消息仍需 @ 机器人（群聊）。
- `/new`、`/sessions` 的话题语义不在本计划范围。
