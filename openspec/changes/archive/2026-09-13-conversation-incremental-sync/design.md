# Design: conversation-incremental-sync

## Context

链路现状：dispatch `transcript_push` 以 `position = log.len()` 无缝分配、
只追加；核心通道已有 `Turns { session_id, from }` 增量变体（
`session_turns(key, from)` 返回 `position >= from`）；webui
`SessionBackend::turns(key, position)` trait 已存在并有"从上次 position
继续"的语义注释。缺口仅在两处：`api.rs::session_detail` 硬编码
`turns(key, 0)`；前端 `dashboard.ts` 每次 WS 事件/动作后 `refetch()` →
`loadFocused` → `sessionDetail` 全量重拉并整体替换 entries。持久化与
重载恢复是既有能力（浏览器验收有场景），本轮不动。

## Goals / Non-Goals

Goals：HTTP `entries_after` 参数、前端内存游标 + 增量 append、
gapless/append-only 契约化。

Non-Goals（proposal 已列）：WS push、localStorage、position 重基、
pending/review-card 增量化。

## Decisions

### D1: 游标 = 已渲染的最大 position，内存 per-session 表

前端 `Map<sessionKey, position>`（普通字段，非持久化）。取值来源：成功
merge 后 `max(旧游标, 新条目最大 position)`；本地序列为空 → 游标视为
无（下次全量）。会话切换只换 focus 不清表——切回已见会话自然增量。
F5 清空整个 Map，天然回到全量。防御性去重：merge 时按 position 过滤
`<= 游标` 的条目（协议保证不会有，防御双触发 refetch 竞态下乱序到达）。

### D2: HTTP 形态 = 现有 detail 端点加 query，不加新端点

`GET /api/sessions/{key}?entries_after=N`：`entries` 切片为
`position > N`，其余字段（status/model face/project binding/encoded_key）
照常返回——status 变化（Working→Done）必须随行，拆独立端点反而让前端
多一次请求。无参数 = 全量（现行为，老消费者零破坏）；非数字 → 400
（不静默当全量，防拼写错误被吞）。实现：axum `Query` 结构 +
`Option<u64>`，透传 `turns(key, after.unwrap_or(0))`——后端协议已支持，
改动约十行。

### D3: merge 顺序与并发

refetch 触发可能重入（WS 事件风暴/动作后 refetch 与 WS refetch 并发）：
merge 按 position 升序插入（新条目本就有序，直接 concat 后按 position
排序一次），游标取 merge 后最大值。后到的增量响应可能包含先到响应已有
的条目（两次 fetch 间游标已推进）——position 过滤幂等吸收。不做请求
去重/取消（成本 > 收益，幂等已兜底）。

### D4: 契约化 gapless/append-only，不加新代码

增量正确性完全依赖「position 无缝 + 只追加」。core 已满足
（`transcript_push` 实现 + `transcript_drop` 仅在会话映射移除时整段
清空，同一会话存续期内无改写路径）。本轮只把该性质写成 webui spec
场景（回归锚），dispatch 侧无改动。

### D5: 未读 seam 与增量共存

unread seam 锚 (position, timestamp)，增量 append 只在尾部追加、不改
既有条目，seam 身份不受影响；`entries_after` 不影响 seam 逻辑（它工作
在已渲染序列上）。

## Risks / Trade-offs

- [重入 refetch 乱序] → D3 幂等 merge 吸收；最坏情况是一次冗余请求。
- [status 与 entries 时点不一致] → 同一 payload 内 status 略新于
  entries（切片窗口内又来了条目），下一次 refetch 收敛；现有全量模式
  同样存在该窗口，非新增问题。
- [`turns` 失败被 `unwrap_or_default` 吞] → 维持现状（渲染空而非失败）；
  游标不推进（空 merge），下次 refetch 自愈。
- [长会话首次全量仍大] → 接受：一次会话一次，后续增量；分页/懒加载是
  另一个变更的事。

## Migration Plan

纯增量演进：无参数行为与今日完全一致，旧消费者（若有）不受影响；无
数据迁移。回滚即回退二进制。

## Open Questions

（无——接口形态、游标生命周期、契约场景均已在拷问中敲定。）
