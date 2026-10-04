// @vitest-environment jsdom
/**
 * 回合终点通知接线单测（fix-webui-qa-round11 3.1，B-3/D3）：turn_engaged
 * true→false 迁移判终态、聚焦/非聚焦两分支、去重与首见帧安静——通知层
 * （notify.ts）的栈上限/去重窗口语义由其自身单测承载，这里只钉接线判定。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { observeTurnFrame, resetTurnNotify, setFocusedSession, type TurnFrame } from './turn-notify.js'
import { resetNotices, subscribeNotices } from '../notify.js'

function frame(overrides: Partial<TurnFrame> = {}): TurnFrame {
  return {
    type: 'session.updated',
    session_id: 'web%00sess-a',
    status_slug: 'working',
    turn_engaged: true,
    msg_count: 2,
    pending: [],
    label: null,
    ...overrides,
  }
}

beforeEach(() => {
  resetNotices()
  resetTurnNotify()
})

describe('turn-notify：回合终点通知（fix-webui-qa-round11 3.1）', () => {
  it('非聚焦会话的占用→空闲迁移弹 info（完成）', () => {
    const seen: string[] = []
    subscribeNotices((s) => {
      seen.length = 0
      seen.push(...s.items.map((i) => `${i.level}:${i.message}`))
    })
    expect(observeTurnFrame(frame({ turn_engaged: true }))).toBe(false)
    expect(observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done' }))).toBe(true)
    expect(seen).toEqual(['info:会话「sess-a」的回合已完成。'])
  })

  it('非聚焦会话的失败迁移弹 error，点名失败', () => {
    let last = ''
    subscribeNotices((s) => {
      if (s.items.length > 0) last = s.items[s.items.length - 1]!.level
    })
    observeTurnFrame(frame({ turn_engaged: true }))
    observeTurnFrame(frame({ turn_engaged: false, status_slug: 'failed' }))
    expect(last).toBe('error')
  })

  it('聚焦中的会话不弹（转录即呈现）', () => {
    setFocusedSession('web%00sess-a')
    observeTurnFrame(frame({ turn_engaged: true }))
    expect(observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done' }))).toBe(false)
    // 焦点换走后同一会话再终点 → 可达。
    setFocusedSession('web%00sess-b')
    expect(observeTurnFrame(frame({ turn_engaged: true }))).toBe(false)
    expect(observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done' }))).toBe(true)
  })

  it('无聚焦（文档路由/卸载投影 null）时终点可达', () => {
    setFocusedSession(null)
    observeTurnFrame(frame({ turn_engaged: true }))
    expect(observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done' }))).toBe(true)
  })

  it('本标签页从未打开过的会话：created 首帧入迁移锚，终点照常弹（fix-webui-qa-round12 1.3）', () => {
    // 另一标签页/另一客户端创建并发起回合；本标签页从加载起就停在历史页、
    // 从未聚焦它——第一帧是 session.created（占用占位），终点帧照样触发。
    const messages: string[] = []
    subscribeNotices((s) => {
      for (const i of s.items) messages.push(i.message)
    })
    expect(
      observeTurnFrame({ ...frame({ turn_engaged: true, status_slug: 'working' }), type: 'session.created' }),
    ).toBe(false)
    expect(observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done' }))).toBe(true)
    expect(messages).toContain('会话「sess-a」的回合已完成。')
  })

  it('首见帧不判终点（无迁移依据），回归占用也不弹', () => {
    expect(observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done' }))).toBe(false)
    // 已知空闲后的空闲帧仍无迁移。
    expect(observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done' }))).toBe(false)
  })

  it('非终态词（queued/working）的迁移安静', () => {
    observeTurnFrame(frame({ turn_engaged: true }))
    expect(observeTurnFrame(frame({ turn_engaged: false, status_slug: 'queued' }))).toBe(false)
  })

  it('同名重复终点在去重窗内不重复弹（dedupeKey 按会话×终态）', () => {
    let count = 0
    subscribeNotices((s) => (count = s.items.length))
    observeTurnFrame(frame({ turn_engaged: true }))
    observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done' }))
    expect(count).toBe(1)
    // 同会话同终态的重复帧（快照收敛重放）被去重。
    observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done' }))
    expect(count).toBe(1)
    // 另一会话的完成是独立通知。
    observeTurnFrame(frame({ session_id: 'web%00sess-b', turn_engaged: true }))
    observeTurnFrame(
      frame({ session_id: 'web%00sess-b', turn_engaged: false, status_slug: 'done' }),
    )
    expect(count).toBe(2)
  })

  it('行名回退链：label 优先、prompt_preview 次之、reference 兜底', () => {
    const messages: string[] = []
    subscribeNotices((s) => {
      for (const i of s.items) messages.push(i.message)
    })
    observeTurnFrame(frame({ turn_engaged: true, label: '我的名字' }))
    observeTurnFrame(frame({ turn_engaged: false, status_slug: 'done', label: '我的名字' }))
    expect(messages).toContain('会话「我的名字」的回合已完成。')

    observeTurnFrame(frame({ session_id: 'web%00sess-b', turn_engaged: true }))
    observeTurnFrame(
      frame({
        session_id: 'web%00sess-b',
        turn_engaged: false,
        status_slug: 'done',
        prompt_preview: '第一条消息',
      }),
    )
    expect(messages).toContain('会话「第一条消息」的回合已完成。')

    observeTurnFrame(frame({ session_id: 'feishu%00agent-53315567', turn_engaged: true }))
    observeTurnFrame(
      frame({
        session_id: 'feishu%00agent-53315567',
        turn_engaged: false,
        status_slug: 'failed',
      }),
    )
    expect(messages).toContain('会话「agent-53315567」的回合失败。')
  })
})
