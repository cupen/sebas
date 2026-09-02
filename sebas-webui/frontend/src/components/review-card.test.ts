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
 * The ElementInternals polyfill is the same shim workbench-composer.test.ts
 * uses: jsdom's ElementInternals lacks setFormValue/setValidity, which the
 * Web Awesome form-associated components call during update.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SebasReviewCards } from './review-card.js'

// ---- ElementInternals polyfill ----------------------------------------

const NOOP_INTERNALS_METHODS = [
  'setFormValue',
  'setValidity',
  'reportValidity',
  'checkValidity',
  'formStateRestoreCallback',
  'formResetCallback',
  'formDisabledCallback',
] as const

const proto = HTMLElement.prototype as unknown as {
  attachInternals?: (this: HTMLElement) => unknown
  __sebasWrappedAttachInternals?: boolean
}
if (!proto.__sebasWrappedAttachInternals) {
  const origAttach = proto.attachInternals
  proto.attachInternals = function (this: HTMLElement): unknown {
    let base: object = {}
    try {
      const r = origAttach?.call(this)
      if (r && typeof r === 'object') base = r as object
    } catch {
      /* non-custom-element or shim rejected */
    }
    const internals: Record<string, unknown> = Object.create(base)
    for (const name of NOOP_INTERNALS_METHODS) {
      if (typeof internals[name] !== 'function') internals[name] = () => {}
    }
    if (!('validity' in internals)) {
      internals.validity = { valid: true, valueMissing: false, customError: false }
    }
    if (!('willValidate' in internals)) internals.willValidate = false
    if (!('labels' in internals)) internals.labels = []
    if (!('form' in internals)) internals.form = null
    return internals
  }
  proto.__sebasWrappedAttachInternals = true
}

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
  api: { answerPermission: vi.fn() },
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
})
