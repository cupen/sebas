/**
 * 回合终点通知接线（fix-webui-qa-round11 3.1，B-3/D3）：会话**非聚焦**时
 * 的回合完成/失败经既有分级通知层弹 info/error 瞬时 toast——操作者停留在
 * 其它页面（历史、用量、另一个会话）也能感知后台回合的终点；聚焦中的会话
 * 其终点由转录本身呈现，不重复弹（spec「聚焦会话不重复通知」）。
 *
 * 职责切分：
 * - **焦点投影**：dashboard（工作台唯一对话面）在每次渲染后把当前聚焦 key
 *   写进 `setFocusedSession`；卸载（切到 /sessions、/usage 等文档路由）时
 *   清空——此刻没有任何聚焦会话，一切终点都该可达。
 * - **帧观测**：app-shell 常驻订阅 `session.created` / `session.updated`
 *   （共享 WS 单连）并喂给 `observeTurnFrame`。终点判定走 **turn_engaged
 *   的 true→false 迁移**（引擎的「回合占用」事实，与 composer 停止钮同一
 *   数据源），终结词取帧上的 `status_slug`（done → info、failed → error）。
 *   首见帧不判终点（无迁移依据，页面刷新后 in-flight 中途开始的帧不算）。
 *
 * 去重沿用通知层既有语义：dedupeKey 按「会话 × 终态」隔离，8s 去重窗内
 * 同一会话同终态的重复帧（快照收敛、事件重放）不刷屏。
 */

import { notify } from '../notify.js'
import type { SessionPhaseFrame } from '../api/ws.js'

/** 观测输入：带 session_id 的相位帧（created / updated 同形）。 */
export type TurnFrame = SessionPhaseFrame & {
  type: 'session.created' | 'session.updated'
  session_id: string
}

let focusedKey: string | null = null
/** encoded key → 上一帧的 turn_engaged（迁移判定锚）。 */
const engaged = new Map<string, boolean>()

/** dashboard 每次渲染后投影当前聚焦 key；null = 无聚焦（文档路由/卸载）。 */
export function setFocusedSession(encodedKey: string | null): void {
  focusedKey = encodedKey
}

/** 仅测试用：清空焦点与迁移锚（模块级 store 在 vitest 用例间要隔离）。 */
export function resetTurnNotify(): void {
  focusedKey = null
  engaged.clear()
}

/**
 * 相位帧入账。弹出终点通知时返回 true（测试断言用），其余情况 false——
 * 首见帧、占用中、非终态词、聚焦中的会话都安静。
 */
export function observeTurnFrame(ev: TurnFrame): boolean {
  if (ev.type !== 'session.created' && ev.type !== 'session.updated') return false
  const key = ev.session_id
  const wasEngaged = engaged.get(key)
  const isEngaged = ev.turn_engaged
  engaged.set(key, isEngaged)
  // 终点 = 占用 → 空闲的迁移；首见帧（wasEngaged === undefined）没有迁移
  // 依据（页面中途打开时回合可能早已在途），不判。
  if (wasEngaged !== true || isEngaged) return false
  const failed = ev.status_slug === 'failed'
  if (!failed && ev.status_slug !== 'done') return false
  // 聚焦中的会话：转录即呈现，不弹（spec 明令）。
  if (key === focusedKey) return false
  const label = ev.label ?? ev.prompt_preview ?? referenceOf(key)
  notify({
    level: failed ? 'error' : 'info',
    message: failed ? `会话「${label}」的回合失败。` : `会话「${label}」的回合已完成。`,
    dedupeKey: `turn.settled:${key}:${ev.status_slug}`,
  })
  return true
}

/** encoded key（`channel%00reference`）尾段的人读形（turn_stalled 同款）。 */
function referenceOf(key: string): string {
  return decodeURIComponent(key.split('%00').pop() ?? key)
}
