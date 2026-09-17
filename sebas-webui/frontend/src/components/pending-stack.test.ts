// @vitest-environment jsdom
/**
 * 待生效堆叠区组件单测（workbench-turn-queue 7.1–7.3；fix-pending-queue-
 * liveness 3.2/3.3 扩展）：
 *   - 7.1 投递序渲染 + 两种处置文案 + 优先项标记；
 *   - 7.2 删除 / 组内重排（键盘按钮 + 拖拽落点换算）+ 乐观对账；
 *     非法落点不发请求（先判后动）；
 *   - 3.2 队列不前进的原因标注：「等待你的审批」/「等待当前回合结束」+
 *     起等时刻（按 turnEngaged / waitingApproval 渲染）；
 *   - 3.3 拒绝反馈两级判据：竞态竞输（服务端真相已收敛）静默对账；
 *     确定性拒绝（条目仍在 / 网络失败）warn 通知点名条目与原因；
 *   - 7.3 会话终结的一次性「未执行」提示。
 *
 * api client 与通知层整体 mock；拖拽用合成 DragEvent 驱动（jsdom 不实现
 * DnD 语义，组件的落点换算逻辑经 dragstart/drop 处理器直接驱动）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { PendingSubmission } from '../api/client.js'

vi.mock('../api/client.js', () => ({
  api: {
    removePending: vi.fn(),
    movePending: vi.fn(),
    session: vi.fn(),
  },
  ApiError: class ApiError extends Error {
    readonly status: number
    constructor(status: number, message: string) {
      super(message)
      this.status = status
    }
  },
  NetworkError: class NetworkError extends Error {},
}))
vi.mock('../notify.js', () => ({ notify: vi.fn() }))

import './pending-stack.js'
import type { SebasPendingStack } from './pending-stack.js'
import { api } from '../api/client.js'
import { notify } from '../notify.js'

const removeMock = api.removePending as ReturnType<typeof vi.fn>
const moveMock = api.movePending as ReturnType<typeof vi.fn>
const sessionMock = api.session as ReturnType<typeof vi.fn>
const notifyMock = notify as ReturnType<typeof vi.fn>

const entries: PendingSubmission[] = [
  { id: 1, text: 'staged one', position: 0, disposition: 'staging', priority: false },
  { id: 2, text: 'staged two', position: 1, disposition: 'staging', priority: false },
  { id: 3, text: 'urgent btw', position: 2, disposition: 'turn', priority: true },
  { id: 4, text: 'queued behind', position: 3, disposition: 'turn', priority: false },
  { id: 5, text: 'queued tail', position: 4, disposition: 'turn', priority: false },
]

async function mount(initial: Partial<SebasPendingStack> = {}): Promise<SebasPendingStack> {
  const el = document.createElement('sebas-pending-stack') as SebasPendingStack
  el.sessionKey = initial.sessionKey ?? 'web%00web-1'
  el.pending = initial.pending ?? entries.map((p) => ({ ...p }))
  if (initial.dropped !== undefined) el.dropped = initial.dropped
  el.turnEngaged = initial.turnEngaged ?? false
  el.waitingApproval = initial.waitingApproval ?? false
  document.body.appendChild(el)
  await el.updateComplete
  return el
}

/** jsdom 没有 DragEvent/DataTransfer——合成带 dataTransfer 存根的通用事件。 */
function dragEvent(type: string): Event {
  const ev = new Event(type, { bubbles: true }) as Event & { dataTransfer?: unknown }
  ev.dataTransfer = { effectAllowed: '', dropEffect: '', setData: () => {} }
  return ev
}

function texts(el: SebasPendingStack): string[] {
  return [...(el.shadowRoot?.querySelectorAll('.entry .text') ?? [])].map(
    (n) => n.textContent ?? '',
  )
}

beforeEach(() => {
  removeMock.mockReset()
  moveMock.mockReset()
  sessionMock.mockReset()
  notifyMock.mockReset()
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('sebas-pending-stack (7.1 rendering)', () => {
  it('lists entries in delivery order with disposition wording', async () => {
    const el = await mount()
    expect(texts(el)).toEqual(['staged one', 'staged two', 'urgent btw', 'queued behind', 'queued tail'])
    const disps = [...(el.shadowRoot?.querySelectorAll('.disp') ?? [])].map(
      (n) => n.textContent ?? '',
    )
    expect(disps[0]).toContain('将并入首条消息')
    expect(disps[1]).toContain('将并入首条消息')
    expect(disps[2]).toContain('待执行 · 第 1 位')
    expect(disps[3]).toContain('待执行 · 第 2 位')
    expect(disps[4]).toContain('待执行 · 第 3 位')
    el.remove()
  })

  it('renders priority entries with a /btw mark and no drag affordance', async () => {
    const el = await mount()
    const priorityEntry = el.shadowRoot?.querySelectorAll('.entry')[2] as HTMLElement
    expect(priorityEntry.querySelector('.prio')).toBeTruthy()
    expect(priorityEntry.getAttribute('draggable')).toBe('false')
    // 优先项没有上移/下移按钮（不可移动），但可删除。
    expect(priorityEntry.querySelector('.mv-up')).toBeNull()
    expect(priorityEntry.querySelector('.mv-down')).toBeNull()
    expect(priorityEntry.querySelector('.remove')).toBeTruthy()
    el.remove()
  })

  it('renders nothing when there is no session, no stack and no notice', async () => {
    const el = document.createElement('sebas-pending-stack') as SebasPendingStack
    el.sessionKey = null
    el.pending = []
    el.dropped = null
    document.body.appendChild(el)
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('.stack')).toBeNull()
    el.remove()
  })
})

describe('sebas-pending-stack (7.2 removal and reorder)', () => {
  it('removes an entry optimistically and reconciles with the server list', async () => {
    const el = await mount()
    removeMock.mockResolvedValue({
      status: 'removed',
      pending: entries.filter((p) => p.id !== 4),
    })
    const btn = el.shadowRoot?.querySelectorAll('.entry')[3]?.querySelector('.remove') as HTMLElement
    btn.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(removeMock).toHaveBeenCalledWith('web%00web-1', 4)
    expect(texts(el)).toEqual(['staged one', 'staged two', 'urgent btw', 'queued tail'])
    el.remove()
  })

  it('silently reconciles when a rejection raced a concurrent start (truth has converged)', async () => {
    const el = await mount()
    // 服务端类型化拒绝（该提交已开始执行），且 post-op 真相里该条目已不在
    // ——竞态竞输：静默对账，绝不弹通知。
    removeMock.mockRejectedValue(new Error('该提交已开始执行'))
    sessionMock.mockResolvedValue({
      pending: entries.filter((p) => p.id !== 4),
    })
    const btn = el.shadowRoot?.querySelectorAll('.entry')[3]?.querySelector('.remove') as HTMLElement
    btn.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    // 无通知；refetch（sebas:refetch → prop 更新）后列表收敛到服务端真相。
    expect(notifyMock).not.toHaveBeenCalled()
    el.pending = entries.filter((p) => p.id !== 4)
    await el.updateComplete
    expect(texts(el)).toEqual(entries.filter((p) => p.id !== 4).map((p) => p.text))
    el.remove()
  })

  it('surfaces a low-severity notice naming entry and reason on a deterministic rejection', async () => {
    const el = await mount()
    // 类型化拒绝（未知条目），post-op 真相里条目仍在 → 确定性拒绝可见。
    removeMock.mockRejectedValue(new Error('待执行提交不存在'))
    sessionMock.mockResolvedValue({ pending: entries.map((p) => ({ ...p })) })
    const btn = el.shadowRoot?.querySelectorAll('.entry')[3]?.querySelector('.remove') as HTMLElement
    btn.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(notifyMock).toHaveBeenCalledTimes(1)
    const arg = notifyMock.mock.calls[0][0] as { level: string; message: string }
    expect(arg.level).toBe('warn')
    expect(arg.message).toContain('移除未生效')
    expect(arg.message).toContain('queued behind')
    expect(arg.message).toContain('待执行提交不存在')
    // 乐观态回滚（条目仍在服务端真相里）。
    expect(texts(el)).toEqual(entries.map((p) => p.text))
    el.remove()
  })

  it('treats a network failure as a deterministic rejection (truth unavailable)', async () => {
    const el = await mount()
    // 网络失败：removePending 与真相取用（api.session）都抛 → 宁可误报，
    // 绝不无感。
    removeMock.mockRejectedValue(new TypeError('fetch failed'))
    sessionMock.mockRejectedValue(new TypeError('fetch failed'))
    const btn = el.shadowRoot?.querySelectorAll('.entry')[3]?.querySelector('.remove') as HTMLElement
    btn.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(notifyMock).toHaveBeenCalledTimes(1)
    const arg = notifyMock.mock.calls[0][0] as { level: string; message: string }
    expect(arg.level).toBe('warn')
    expect(arg.message).toContain('移除未生效')
    el.remove()
  })

  it('notices when a removal claims success but the entry is still in the server list', async () => {
    const el = await mount()
    // 2xx 但全量里条目仍在 = 服务端没有执行该操作 → 确定性拒绝可见。
    removeMock.mockResolvedValue({ status: 'removed', pending: entries.map((p) => ({ ...p })) })
    const btn = el.shadowRoot?.querySelectorAll('.entry')[3]?.querySelector('.remove') as HTMLElement
    btn.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(notifyMock).toHaveBeenCalledTimes(1)
    const arg = notifyMock.mock.calls[0][0] as { level: string; message: string }
    expect(arg.message).toContain('服务端未执行该移除')
    el.remove()
  })

  it('keyboard move reorders within the turn group and reconciles with the server list', async () => {
    const el = await mount()
    moveMock.mockResolvedValue({
      status: 'moved',
      pending: [entries[0], entries[1], entries[2], entries[4], entries[3]].map((p, i) => ({
        ...p,
        position: i,
      })),
    })
    // queued tail（组内第 2 位，0 基）上移一位。
    const up = el.shadowRoot?.querySelectorAll('.entry')[4]?.querySelector('.mv-up') as HTMLElement
    up.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(moveMock).toHaveBeenCalledWith('web%00web-1', 5, 1)
    // 服务端对账：位置不变（id 5 仍在 tail 位），以服务端返回重建。
    expect(texts(el)).toEqual(['staged one', 'staged two', 'urgent btw', 'queued tail', 'queued behind'])
    el.remove()
  })

  it('refuses an illegal drag target without calling the API (judge before move)', async () => {
    const el = await mount()
    // 把普通项拖到优先项上 → 组内落点 0 = 越过优先项 → 不发请求、不乐观。
    const dragged = el.shadowRoot?.querySelectorAll('.entry')[3] as HTMLElement
    const target = el.shadowRoot?.querySelectorAll('.entry')[2] as HTMLElement
    dragged.dispatchEvent(dragEvent('dragstart'))
    target.dispatchEvent(dragEvent('dragover'))
    target.dispatchEvent(dragEvent('drop'))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(moveMock).not.toHaveBeenCalled()
    expect(texts(el)).toEqual(entries.map((p) => p.text))
    el.remove()
  })

  it('maps a legal drag onto the group-local index and reorders optimistically', async () => {
    const el = await mount()
    moveMock.mockResolvedValue({
      status: 'moved',
      pending: [
        entries[0],
        entries[1],
        entries[2],
        entries[4],
        entries[3],
      ].map((p, i) => ({ ...p, position: i })),
    })
    // queued behind（组内 1）拖到 queued tail（组内 2）→ to_index 2。
    const dragged = el.shadowRoot?.querySelectorAll('.entry')[3] as HTMLElement
    const target = el.shadowRoot?.querySelectorAll('.entry')[4] as HTMLElement
    dragged.dispatchEvent(dragEvent('dragstart'))
    target.dispatchEvent(dragEvent('drop'))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(moveMock).toHaveBeenCalledWith('web%00web-1', 4, 2)
    expect(texts(el)).toEqual(['staged one', 'staged two', 'urgent btw', 'queued tail', 'queued behind'])
    el.remove()
  })
})

describe('sebas-pending-stack (7.3 not-executed notice)', () => {
  it('renders a one-time notice naming the dropped entries', async () => {
    const el = await mount()
    el.dropped = [
      { id: 4, text: 'queued behind', position: 0, disposition: 'turn', priority: false },
      { id: 5, text: 'queued tail', position: 1, disposition: 'turn', priority: false },
    ]
    await el.updateComplete
    const notice = el.shadowRoot?.querySelector('[role="alert"]')
    expect(notice).toBeTruthy()
    expect(notice!.textContent).toContain('未被执行')
    expect(notice!.textContent).toContain('queued behind')
    expect(notice!.textContent).toContain('queued tail')
    el.remove()
  })
})

describe('sebas-pending-stack (fix-pending-queue-liveness 3.2 wait reason)', () => {
  it('keeps bare position wording while the turn is not engaged', async () => {
    const el = await mount()
    const disps = [...(el.shadowRoot?.querySelectorAll('.disp') ?? [])].map(
      (n) => n.textContent ?? '',
    )
    expect(disps[2]).toContain('待执行 · 第 1 位')
    el.remove()
  })

  it('names the running turn as the blocker with the wait start when engaged', async () => {
    const el = await mount({ turnEngaged: true })
    // 锚点拨到 65s 前 → 「已等 1 分钟」（分桶呈现，不假精度）。
    el.blockedSince = Date.now() - 65_000
    await el.updateComplete
    const disps = [...(el.shadowRoot?.querySelectorAll('.disp') ?? [])].map(
      (n) => n.textContent ?? '',
    )
    expect(disps[2]).toContain('等待当前回合结束')
    expect(disps[2]).toContain('第 1 位')
    expect(disps[2]).toContain('已等 1 分钟')
    el.remove()
  })

  it('names the owed permission decision as the blocker while parked', async () => {
    const el = await mount({ turnEngaged: true, waitingApproval: true })
    el.blockedSince = Date.now() - 5_000
    await el.updateComplete
    const disps = [...(el.shadowRoot?.querySelectorAll('.disp') ?? [])].map(
      (n) => n.textContent ?? '',
    )
    expect(disps[2]).toContain('等待你的审批')
    expect(disps[2]).toContain('已等 5 秒')
    // staging 条目的语义不变（并入首条消息与阻塞无关）。
    expect(disps[0]).toContain('将并入首条消息')
    el.remove()
  })
})
