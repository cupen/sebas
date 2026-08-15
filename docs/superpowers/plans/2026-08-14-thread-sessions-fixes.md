# 话题（thread）会话设计复审修复 Plan

> **For agentic workers:** 按 Task 1→4 顺序执行，每个 Task 完成后跑对应测试再进下一个。
> 完成后 `cargo build && cargo test` 全绿，改动提交在 `feat/thread-sessions-fixes` 分支（已建好，基于 main tip b65b02f），禁止 push、禁止动 main。

**背景**：2026-08-14 复审了 `2026-08-09-feishu-thread-sessions.md` 的设计与实现，发现 6 个问题。本 plan 覆盖其中 4 个可纯代码修复的项；另外 2 个（`reply_in_thread` 是否必须、p2p 引用回复是否带 thread_id）是飞书行为假设，需云端实测，不在本 plan 内。

**涉及文件**：

| 文件 | 相关位置 |
|---|---|
| `src/dispatch.rs` | `send_card_topic_aware`（:39）、`TOPIC_INVALID_NOTICE`（:19）、`topic_reply_target`（:25）、`dispatch_out`（:71） |
| `router/src/router/mod.rs` | `web_close_session`（:598，`CloseOutcome` :122）、`reply_target()` getter（:500） |
| `router/src/router/inbound.rs` | `spawn_new`（:365，`allowlist.clear` 在 :369） |
| `router/src/router/acp_events.rs` | 权限卡 root_id 预填（:62-66）、terminal Error 清理（:95-99） |
| `router/src/router/maps.rs` | `ReplyTargetMap`（:164） |
| `feishu/src/client.rs` | `FeishuApiError::is_topic_invalid`（:38）、`send_text`（:275） |

## Task 1：ReplyTargetMap 清理收口（F2，最简单，先做）

**问题**：`ReplyTargetMap::clear` 只挂在 `acp_events.rs:97`（terminal Error）一处；`spawn_new`（inbound.rs:365）和 `web_close_session`（mod.rs:598）都不清 → map 随话题数无界增长。

**改动**：
- `spawn_new`：在 `allowlist.clear(&key)`（inbound.rs:369）旁加 `self.reply_targets.clear(&key).await;`。
- `web_close_session`：在 `self.allowlist.clear(&key).await;`（mod.rs:639）旁加 `self.reply_targets.clear(&key).await;`。
- 不动 allowlist 的既有清理模式（那是另一个问题，本次不重构）。

**测试**：router 层新增/更新单测：
- `spawn_new` 后 `reply_target(&key)` 返回 `None`（先 set 再 spawn_new）。
- `web_close_session` 后 `reply_target(&key)` 返回 `None`。

## Task 2：去掉权限卡 root_id 双重填充（F3）

**问题**：`acp_events.rs:62-66` 预填一次 `Out::SendCard.root_id`；`dispatch.rs:96` 的 `topic_reply_target` 又对空 root_id 兜底再查一次。同一职责两处实现，会漂移。

**改动**：
- `acp_events.rs`：删除 :62-66 的条件查询，权限卡 `root_id` 直接填 `None`，注释改为说明"话题回复目标由 dispatch 层 `topic_reply_target` 统一兜底"。
- `dispatch.rs` 的兜底逻辑不变（它是唯一收口）。

**测试**：更新 router 层权限卡用例的断言：话题内权限请求发出的 `Out::SendCard.root_id == None`（原来断言 == 话题根消息的要改）。dispatch 层兜底已有测试则确认仍绿。

## Task 3：话题失效熔断（F1，核心）

**问题**：`send_card_topic_aware` 命中 230019/230071 后只发提示、返回空 id，会话继续活着 → 后续每张出站卡再失败一次再刷一条提示，ACP 子代理空烧 token。

**改动**：
- `send_card_topic_aware` 改为返回枚举而不是 `String`，例如：
  ```rust
  pub(crate) enum TopicSendOutcome {
      Sent(String),      // message_id
      TopicInvalid,      // 已熔断，调用方按"未发出"处理
  }
  ```
  （保持 `anyhow::Result<TopicSendOutcome>`，其他错误照常冒泡。）
- 命中 topic-invalid 时：发一次文本提示 + 调 `router.web_close_session(key.clone()).await` 终止会话（它会 kill ACP 子进程、清 SessionMap/CardState/MsgIdMap/allowlist，Task 1 之后也清 ReplyTargetMap）。注意 `send_card_topic_aware` 目前没有 `router` 参数——把 `&RouterHandle` 加进参数列表，调用点（dispatch.rs:97、session_boot.rs:183 附近）同步更新。
- 熔断后会话映射已删，后续入站会走"无会话"路径（session-lost 或重新 spawn），自然不会再向失效话题出站 → 提示只发一次，不需要额外的去重标记。
- 文案按会话类型区分：`key.thread_id.is_some()` 且是群聊无法在此区分 p2p——简单做法：文案改为「该话题已失效，本次会话已结束。请重新发消息开始新会话。」（群聊/p2p 通用，不提"开新话题"）。

**注意**：
- `web_close_session` 对无会话的 key 返回 `CloseOutcome` 的非 Closed 变体，熔断路径忽略返回值即可（幂等）。
- `dispatch_out` 里 `Out::SendCard` 分支对 `TopicInvalid` 的处理等价于现在的空 id（跳过 record_root_msg_id / record_perm_card_msg_id），match 重构时保持这个语义。
- session_boot.rs 的初始卡路径同样走熔断（首次出站就发现话题失效，更要终止）。

**测试**：
- 构造 `FeishuApiError{code:230019}` 的 mock/feishu client 替身（看现有 client 测试怎么模拟，feishu/src/client.rs 有 230019 的 downcast 测试可参考）→ 断言：返回 `TopicInvalid`、`web_close_session` 被调（SessionMap 映射消失）、提示文本发出。
- 230071 同路径；非话题错误码（如 99999）→ 错误照常冒泡、不熔断。

## Task 4：收尾

- 全量 `cargo build`（dev，禁止 --release）+ `cargo test` 全绿。
- `cargo clippy` 如有新增 warning 一并处理（既有 warning 不动）。
- 提交：Conventional Commits，能一行说完就一行；建议 Task 1+2 一个 commit（都是清理收口），Task 3 一个 commit。
- 在 plan 文件末尾追加一节「执行记录」：每个 Task 实际改动文件、测试结果、偏差说明。

## 明确不做（禁止顺手改）

- `reply_in_thread` 参数、出站结构改动 —— 待云端实测。
- p2p 引用回复 vs 话题的语义区分 —— 待云端实测。
- ReplyTargetMap 持久化（重启窗口）—— 单独评估。
- `allowlist.clear` 散落模式重构、`SessionAllowlist` 任何行为变更。
- `/new`、`/sessions` 的话题语义。

## 验收清单

- [ ] `cargo test` 全绿（含新增用例）
- [ ] 话题失效 → 会话被终止、提示只发一次
- [ ] `spawn_new` / `web_close_session` 后 reply_target 被清
- [ ] 权限卡 `Out::SendCard.root_id` 恒为 `None`，话题聚合由 dispatch 兜底
- [ ] 提交在 `feat/thread-sessions-fixes`，main 无改动

## 执行记录

**2026-08-15 完成**（`feat/thread-sessions-fixes`，基于新 main `4ab3859` rebase 后）：

| Task | 改动 | 测试 |
|---|---|---|
| 1 (F2) ReplyTargetMap 清理 | `inbound.rs` spawn_new 旁 `reply_targets.clear`；`mod.rs` web_close_session 旁 `reply_targets.clear`；`routing_paths_test.rs`、`web_close_test.rs` 新增用例 | 绿 |
| 2 (F3) 权限卡 root_id 去双重填充 | `acp_events.rs` 删除预填，恒 `None`；`permission_test.rs` 断言改为恒 None；dispatch 兜底不动 | 绿 |
| 3 (F1) 话题失效熔断 | `dispatch.rs` `TopicSendOutcome` 枚举 + `classify_topic_invalid` + 熔断调 `web_close_session`；`session_boot.rs` 初始卡同走熔断；新增 3 个 dispatch 单测 | 绿 |
| 4 (收尾) | `cargo build`（dev）绿；`cargo test` 全绿（除 2 个 main 上已存在的失败，见下）；clippy 无新增 warning | — |

**提交**（main..HEAD，5 个）：
- `3d8b2d8` feat: thread-scoped sessions（原 aba52f0，rebase 冲突解决）
- `31b8ae8` fix: reply target 清理 + 权限卡 root_id（Task 1+2）
- `cec5cfe` fix: 话题失效熔断（Task 3）
- `73b36d6` fix: 适配 main 的 render_accumulated_card 新签名（去掉 seed_emoji 参数）
- `314ed29` fix: mainline root 卡回退到 input_msg_id（main 合并后语义调和）

**偏差说明**：
- rebase 冲突：`session_boot.rs`（main 引入 `input_msg_id` + 卡片标题改造 vs 话题会话初始卡）——解决为 `topic_reply_target(...).or(input_msg_id)`：话题会话回复话题根，主线回退到用户输入消息（保留 main 新行为）。
- 未 push、未动 main。
- main 上已存在、与本分支无关的失败（未修复）：`watchdog::control_rpc::default_socket_path_ends_with_control_sock`（XDG_RUNTIME_DIR 未设置时断言路径后缀）；`full_e2e_test::slow_stream_*`（时序敏感，~10% flake）。
