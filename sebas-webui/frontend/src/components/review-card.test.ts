// @vitest-environment jsdom
/**
 * sebas-review-cards behaviour. The shared WebSocket client and the api
 * client are mocked; frames are fanned out to the component directly so we
 * can drive the full review loop:
 *   - a permission.requested frame renders one card (tool/reason/session/args)
 *   - duplicate frames never create a second card (dedup by request_id)
 *   - allow once / allow session / deny answer with the right decision body
 *     and remove the card
 *   - escalate sends the typed reason
 *   - 404 marks the card expired and inert (no re-answer)
 *   - other errors keep the card retryable with the failure surfaced
 *   - sessionKey filters frames and switching keys clears collected cards
 *
 * The ElementInternals polyfill is shared with the other WA-rendering tests
 * (see test-support/wa-polyfills.ts): jsdom's ElementInternals lacks
 * setFormValue/setValidity, which the Web Awesome form-associated components
 * call during update.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SebasReviewCards } from './review-card.js'
import { installWaDomPolyfills } from '../test-support/wa-polyfills.js'

// ---- ElementInternals polyfill（共享垫片）--------------------------------

installWaDomPolyfills()

if (typeof (globalThis as { ResizeObserver?: unknown }).ResizeObserver === 'undefined') {
  class StubResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  ;(globalThis as unknown as { ResizeObserver: typeof StubResizeObserver }).ResizeObserver =
    StubResizeObserver
}

// ---- module mocks ------------------------------------------------------

/** Test-facing surface of the mocked shared WS singleton. */
interface FakeSharedWs {
  subscribe(handler: (e: unknown) => void): () => void
  /** Fan a frame out to every subscriber (views + the component). */
  emit(frame: unknown): void
  reset(): void
}

vi.mock('../api/shared-ws.js', () => {
  const handlers = new Set<(e: unknown) => void>()
  return {
    sharedWs: {
      subscribe(handler: (e: unknown) => void): () => void {
        handlers.add(handler)
        return () => handlers.delete(handler)
      },
      emit(frame: unknown): void {
        for (const handler of handlers) handler(frame)
      },
      reset(): void {
        handlers.clear()
      },
    },
  }
})

vi.mock('../api/client.js', () => ({
  api: { answerPermission: vi.fn(), sessionApprovals: vi.fn() },
  ApiError: class MockApiError extends Error {
    readonly status: number
    constructor(status: number, message: string) {
      super(message)
      this.status = status
    }
  },
}))

// Component import — registers <sebas-review-cards> with the mocks above
// and the WA side-effects (wa-button / wa-input) in place.
import './review-card.js'
import { sharedWs } from '../api/shared-ws.js'
import { api, ApiError } from '../api/client.js'

const ws = sharedWs as unknown as FakeSharedWs
const answerMock = api.answerPermission as ReturnType<typeof vi.fn>
const approvalsMock = api.sessionApprovals as ReturnType<typeof vi.fn>

// ---- helpers -----------------------------------------------------------

interface PermFrame {
  type: 'permission.requested'
  request_id: string
  session_id: string
  tool_name: string
  args: unknown
  reason: string
}

function permFrame(overrides: Partial<PermFrame> = {}): PermFrame {
  return {
    type: 'permission.requested',
    request_id: 'toolu_1',
    session_id: 'oc_enc',
    tool_name: 'bash',
    args: { command: 'rm -rf build' },
    reason: 'may modify state',
    ...overrides,
  }
}

async function mount(sessionKey: string | null = null): Promise<SebasReviewCards> {
  const el = document.createElement('sebas-review-cards') as SebasReviewCards
  if (sessionKey !== null) el.sessionKey = sessionKey
  document.body.appendChild(el)
  await el.updateComplete
  return el
}

async function flush(el: SebasReviewCards): Promise<void> {
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
}

function cards(el: SebasReviewCards): NodeListOf<HTMLElement> {
  return el.shadowRoot!.querySelectorAll<HTMLElement>('.review-card')
}

beforeEach(() => {
  vi.clearAllMocks()
  ws.reset()
  answerMock.mockResolvedValue({ status: 'delivered' })
  approvalsMock.mockResolvedValue({ approvals: [] })
})

afterEach(() => {
  document.body.innerHTML = ''
})

// ---- tests -------------------------------------------------------------

describe('sebas-review-cards', () => {
  it('renders nothing until a frame arrives, then one card per frame', async () => {
    const el = await mount()
    expect(cards(el).length).toBe(0)
    ws.emit(permFrame())
    await el.updateComplete
    expect(cards(el).length).toBe(1)

    const card = cards(el)[0]!
    expect(card.dataset.requestId).toBe('toolu_1')
    expect(card.querySelector('.tool')?.textContent).toBe('bash')
    expect(card.querySelector('.why')?.textContent).toContain('may modify state')
    expect(card.querySelector('.session-id')?.textContent).toContain('session oc_enc')
    expect(card.querySelector('.request-id')?.textContent).toContain('request toolu_1')
    // Args are rendered as formatted JSON (2-space indent).
    expect(card.querySelector('.args')?.textContent).toContain('"command": "rm -rf build"')
  })

  it('deduplicates repeated frames by request_id', async () => {
    const el = await mount()
    ws.emit(permFrame())
    ws.emit(permFrame()) // same request_id again (broadcast / reconnect replay)
    ws.emit(permFrame({ request_id: 'toolu_2' }))
    await el.updateComplete
    expect(cards(el).length).toBe(2)
    expect([...cards(el)].map((c) => c.dataset.requestId)).toEqual(['toolu_1', 'toolu_2'])
  })

  it('allow once answers and removes the card', async () => {
    const el = await mount()
    ws.emit(permFrame())
    await el.updateComplete

    const button = cards(el)[0]!.querySelector('wa-button.allow-once')
    expect(button).toBeTruthy()
    ;(button as HTMLElement).click()
    await flush(el)

    expect(answerMock).toHaveBeenCalledTimes(1)
    expect(answerMock).toHaveBeenCalledWith('toolu_1', { decision: 'allow_once' })
    expect(cards(el).length).toBe(0)
  })

  it('allow session and deny answer with their decision bodies', async () => {
    const el = await mount()
    ws.emit(permFrame({ request_id: 'toolu_1' }))
    ws.emit(permFrame({ request_id: 'toolu_2' }))
    await el.updateComplete

    ;(cards(el)[0]!.querySelector('wa-button.allow-session') as HTMLElement).click()
    ;(cards(el)[1]!.querySelector('wa-button.deny') as HTMLElement).click()
    await flush(el)

    expect(answerMock).toHaveBeenCalledTimes(2)
    expect(answerMock).toHaveBeenNthCalledWith(1, 'toolu_1', { decision: 'allow_session' })
    expect(answerMock).toHaveBeenNthCalledWith(2, 'toolu_2', { decision: 'deny' })
    expect(cards(el).length).toBe(0)
  })

  it('escalate sends the typed reason and is gated on non-empty input', async () => {
    const el = await mount()
    ws.emit(permFrame())
    await el.updateComplete

    const card = cards(el)[0]!
    const escalateBtn = card.querySelector('wa-button.escalate') as HTMLElement
    // No reason typed yet → the button renders disabled (wa gate).
    expect(escalateBtn.hasAttribute('disabled')).toBe(true)

    const input = card.querySelector('wa-input.escalate-reason') as unknown as {
      value: string
    }
    input.value = 'need network for the install'
    ;(input as unknown as HTMLElement).dispatchEvent(
      new Event('input', { bubbles: true, composed: true }),
    )
    await el.updateComplete
    expect(escalateBtn.hasAttribute('disabled')).toBe(false)

    escalateBtn.click()
    await flush(el)

    expect(answerMock).toHaveBeenCalledTimes(1)
    expect(answerMock).toHaveBeenCalledWith('toolu_1', {
      decision: 'escalate',
      reason: 'need network for the install',
    })
    expect(cards(el).length).toBe(0)
  })

  it('marks the card expired on 404 and refuses further answers', async () => {
    answerMock.mockRejectedValue(new ApiError(404, 'no pending permission request with that id'))
    const el = await mount()
    ws.emit(permFrame())
    await el.updateComplete

    const button = cards(el)[0]!.querySelector('wa-button.allow-once') as HTMLElement
    button.click()
    await flush(el)

    // The card stays visible but inert: the action row is replaced by the
    // expired callout.
    expect(cards(el).length).toBe(1)
    const card = cards(el)[0]!
    expect(card.dataset.state).toBe('expired')
    expect(card.querySelector('.callout-warning')?.textContent).toContain('No longer pending')
    expect(card.querySelector('wa-button.allow-once')).toBeNull()

    // A late repeat of the same frame cannot resurrect the expired card,
    // so there is nothing left to answer and no second POST fires.
    ws.emit(permFrame())
    await el.updateComplete
    expect(cards(el).length).toBe(1)
    expect(answerMock).toHaveBeenCalledTimes(1)
  })

  it('keeps the card retryable on non-404 errors with the failure surfaced', async () => {
    answerMock.mockRejectedValueOnce(new ApiError(500, 'boom'))
    const el = await mount()
    ws.emit(permFrame())
    await el.updateComplete

    ;(cards(el)[0]!.querySelector('wa-button.allow-once') as HTMLElement).click()
    await flush(el)

    const card = cards(el)[0]!
    expect(card.dataset.state).toBe('pending')
    expect(card.querySelector('.callout-error')?.textContent).toContain('boom')
    // Retry succeeds.
    answerMock.mockResolvedValueOnce({ status: 'delivered' })
    ;(card.querySelector('wa-button.allow-once') as HTMLElement).click()
    await flush(el)
    expect(answerMock).toHaveBeenCalledTimes(2)
    expect(cards(el).length).toBe(0)
  })

  // ---- fix-webui-approval-restore-and-session-identity 1.3：读模型重建 ----

  it('rebuilds the review surface from the read model when sessionKey is set', async () => {
    approvalsMock.mockResolvedValue({
      approvals: [
        {
          request_id: 'tc_rebuild',
          tool_name: 'Write',
          args: { path: '/proj/a.rs' },
        },
      ],
    })
    const el = await mount('oc_enc')
    await flush(el)

    expect(api.sessionApprovals).toHaveBeenCalledWith('oc_enc')
    expect(cards(el).length).toBe(1)
    const card = cards(el)[0]!
    expect(card.dataset.requestId).toBe('tc_rebuild')
    expect(card.querySelector('.tool')?.textContent).toBe('Write')
    expect(card.querySelector('wa-button.allow-once')).toBeTruthy()
  })

  it('merges a push for an already-rebuilt request_id into the same card', async () => {
    approvalsMock.mockResolvedValue({
      approvals: [{ request_id: 'tc_dup', tool_name: 'Bash', args: {} }],
    })
    const el = await mount('oc_enc')
    await flush(el)
    expect(cards(el).length).toBe(1)

    // The broadcast repeats the same id (the rebuild raced the push) —
    // exactly one decision surface, no duplicate.
    ws.emit(permFrame({ request_id: 'tc_dup', session_id: 'oc_enc' }))
    await el.updateComplete
    expect(cards(el).length).toBe(1)
    expect(cards(el)[0]!.dataset.requestId).toBe('tc_dup')
  })

  it('never resurrects a decided id from a push or a stale read-model row', async () => {
    approvalsMock.mockResolvedValue({
      approvals: [{ request_id: 'tc_gone', tool_name: 'Bash', args: {} }],
    })
    const el = await mount('oc_enc')
    await flush(el)
    expect(cards(el).length).toBe(1)

    ;(cards(el)[0]!.querySelector('wa-button.allow-once') as HTMLElement).click()
    await flush(el)
    expect(cards(el).length).toBe(0)

    // Late broadcast for the decided id: no card.
    ws.emit(permFrame({ request_id: 'tc_gone', session_id: 'oc_enc' }))
    await el.updateComplete
    expect(cards(el).length).toBe(0)

    // A stale read-model re-pull still listing the decided id: no card.
    await el['pullApprovals']('oc_enc')
    await flush(el)
    expect(cards(el).length).toBe(0)
  })

  it('ignores frames for other sessions when sessionKey is set, clears on switch', async () => {
    const el = await mount('oc_a')
    ws.emit(permFrame({ request_id: 't_other', session_id: 'oc_b' }))
    await el.updateComplete
    expect(cards(el).length).toBe(0)

    ws.emit(permFrame({ request_id: 't_a', session_id: 'oc_a' }))
    await el.updateComplete
    expect(cards(el).length).toBe(1)

    // Switching the viewed session drops the previous session's cards.
    el.sessionKey = 'oc_b'
    await el.updateComplete
    expect(cards(el).length).toBe(0)

    // …and frames for the newly viewed session render again.
    ws.emit(permFrame({ request_id: 't_b', session_id: 'oc_b' }))
    await el.updateComplete
    expect(cards(el).length).toBe(1)
    expect(cards(el)[0]!.dataset.requestId).toBe('t_b')
  })

  // ---- fix-webui-qa-defects-round3 2.1/2.2：单一 store 入口 + 相位对账 ----

  it('renders the review card the moment the push arrives (no reload)', async () => {
    // 固定期望行为（2.1）：推送到达即渲染——这是缺陷 2 的验收句，结构收敛
    // （2.2）不得改变它。
    const el = await mount('oc_enc')
    expect(cards(el).length).toBe(0)
    ws.emit(permFrame({ request_id: 'tc_push', session_id: 'oc_enc' }))
    await el.updateComplete
    expect(cards(el).length).toBe(1)
    expect(cards(el)[0]!.dataset.requestId).toBe('tc_push')
  })

  it('reconciles with the read model when the phase says waiting but no live card exists', async () => {
    // 丢帧自愈（2.2）：rail 已亮「等待」（相位帧 waiting），但推送丢了——
    // 相位变化触发读模型重取（同一 mergeRows 入口），卡片不必等 reload。
    approvalsMock.mockResolvedValue({ approvals: [] })
    const el = await mount('oc_enc')
    await flush(el)
    expect(cards(el).length).toBe(0)

    // 此刻读模型里已有泊车审批（重启后重拉才可见），且再无推送到来。
    approvalsMock.mockResolvedValue({
      approvals: [{ request_id: 'tc_heal', tool_name: 'Bash', args: {} }],
    })
    el.sessionPhase = 'waiting'
    await flush(el)
    expect(cards(el).length).toBe(1)
    expect(cards(el)[0]!.dataset.requestId).toBe('tc_heal')
    expect(approvalsMock).toHaveBeenCalledWith('oc_enc')
  })

  it('phase reconciliation does not duplicate a live card or resurrect a decided one', async () => {
    approvalsMock.mockResolvedValue({
      approvals: [{ request_id: 'tc_live', tool_name: 'Bash', args: {} }],
    })
    const el = await mount('oc_enc')
    el.sessionPhase = 'waiting'
    await flush(el)
    expect(cards(el).length).toBe(1)

    // 相位再次翻到 waiting（帧重放）：store 已有待决卡 → 不重取不重复。
    approvalsMock.mockClear()
    el.sessionPhase = 'working'
    await el.updateComplete
    el.sessionPhase = 'waiting'
    await flush(el)
    expect(cards(el).length).toBe(1)
    expect(approvalsMock).not.toHaveBeenCalled()

    // 已决策的卡：相位再回 waiting，读模型里陈旧行也不复活。
    ;(cards(el)[0]!.querySelector('wa-button.allow-once') as HTMLElement).click()
    await flush(el)
    expect(cards(el).length).toBe(0)
    approvalsMock.mockResolvedValue({
      approvals: [{ request_id: 'tc_live', tool_name: 'Bash', args: {} }],
    })
    el.sessionPhase = 'working'
    await el.updateComplete
    el.sessionPhase = 'waiting'
    await flush(el)
    expect(cards(el).length).toBe(0)
  })

  it('phase reconciliation is inert without a session key or on non-waiting phases', async () => {
    approvalsMock.mockResolvedValue({
      approvals: [{ request_id: 'tc_noop', tool_name: 'Bash', args: {} }],
    })
    const el = await mount(null)
    el.sessionPhase = 'waiting'
    await flush(el)
    // sessionKey 为空（渲染所有会话的面）：没有可对账的读模型入口，不动。
    expect(approvalsMock).not.toHaveBeenCalled()
    expect(cards(el).length).toBe(0)

    el.sessionPhase = 'working'
    await el.updateComplete
    expect(approvalsMock).not.toHaveBeenCalled()
  })
})
