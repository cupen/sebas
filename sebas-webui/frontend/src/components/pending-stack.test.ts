// @vitest-environment jsdom
/**
 * 待生效堆叠区组件单测（workbench-turn-queue 7.1–7.3）：
 *   - 7.1 投递序渲染 + 两种处置文案 + 优先项标记；
 *   - 7.2 删除 / 组内重排（键盘按钮 + 拖拽落点换算）+ 乐观对账 +
 *     AlreadyStarted 等拒绝的静默刷新；非法落点不发请求（先判后动）；
 *   - 7.3 会话终结的一次性「未执行」提示。
 *
 * api client 整体 mock；拖拽用合成 DragEvent 驱动（jsdom 不实现 DnD 语义，
 * 组件的落点换算逻辑经 dragstart/drop 处理器直接驱动）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { PendingSubmission } from '../api/client.js'

vi.mock('../api/client.js', () => ({
  api: {
    removePending: vi.fn(),
    movePending: vi.fn(),
  },
}))

import './pending-stack.js'
import type { SebasPendingStack } from './pending-stack.js'
import { api } from '../api/client.js'

const removeMock = api.removePending as ReturnType<typeof vi.fn>
const moveMock = api.movePending as ReturnType<typeof vi.fn>

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

  it('silently reconciles (no error UI) when the server rejects with AlreadyStarted', async () => {
    const el = await mount()
    removeMock.mockRejectedValue(new Error('该提交已开始执行'))
    const btn = el.shadowRoot?.querySelectorAll('.entry')[3]?.querySelector('.remove') as HTMLElement
    btn.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    // 乐观态回滚、无错误弹层；组件不崩、列表恢复服务端真相。
    expect(el.shadowRoot?.querySelector('.callout-error')).toBeNull()
    expect(texts(el)).toEqual(entries.map((p) => p.text))
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
