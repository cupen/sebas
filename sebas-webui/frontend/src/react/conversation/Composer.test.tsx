// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ConversationEntryView, SessionDetail } from '../../api/client.js'
import { Composer } from './Composer.js'
import { TranscriptView } from './TranscriptView.js'

afterEach(cleanup)

const detail: SessionDetail = {
  chat_id: 'c1',
  thread_id: null,
  session_id: 's1',
  status: 'done',
  status_label: 'done',
  status_slug: 'done',
  status_glyph: '',
  entries: [],
  msg_id: null,
  last_active: '',
  encoded_key: 'oc_1',
  msg_count: 0,
  current_model: 'm1',
  available_models: ['m1', 'm2'],
  agent_kind: 'codex',
  desired_mode: 'ask',
  pending: [],
  turn_engaged: false,
  available_commands: [{ name: 'goal', description: 'set goal', hint: '<text>' }],
}

function entry(patch: Partial<ConversationEntryView>): ConversationEntryView {
  return {
    position: 0,
    kind: 'content',
    element_type: 'markdown',
    content: 'hello',
    created_at_unix: 0,
    ...patch,
  }
}

describe('Composer', () => {
  it('sends ordinary text and clears the draft', async () => {
    const send = vi.fn(async () => ({}))
    const refresh = vi.fn(async () => {})
    render(
      <Composer sessionKey="oc_1" detail={detail} reachability={{ ok: true }} onRefresh={refresh} send={send} />,
    )
    fireEvent.change(screen.getByLabelText('输入消息'), { target: { value: 'do the thing' } })
    fireEvent.click(screen.getByRole('button', { name: /发\s*送/ }))
    expect(send).toHaveBeenCalledWith('oc_1', 'do the thing')
    await waitFor(() =>
      expect((screen.getByLabelText('输入消息') as HTMLTextAreaElement).value).toBe(''),
    )
  })

  it('blocks an unadvertised slash command without sending', () => {
    const send = vi.fn()
    render(
      <Composer sessionKey="oc_1" detail={detail} reachability={{ ok: true }} onRefresh={() => {}} send={send} />,
    )
    fireEvent.change(screen.getByLabelText('输入消息'), { target: { value: '/nope' } })
    fireEvent.click(screen.getByRole('button', { name: /发\s*送/ }))
    expect(send).not.toHaveBeenCalled()
    expect(screen.getByText(/不支持此命令/)).toBeTruthy()
  })

  it('shows the stop control for an empty draft during a turn', () => {
    const cancel = vi.fn(async () => ({}))
    render(
      <Composer
        sessionKey="oc_1"
        detail={{ ...detail, turn_engaged: true }}
        reachability={{ ok: false, cause: 'down' }}
        onRefresh={() => {}}
        cancel={cancel}
      />,
    )
    const button = screen.getByRole('button', { name: /停\s*止/ })
    expect(button.getAttribute('data-state')).toBe('stop')
    fireEvent.click(button)
    expect(cancel).toHaveBeenCalledWith('oc_1')
  })

  it('renders the pending queue and moves an item', () => {
    const move = vi.fn(async () => ({}))
    render(
      <Composer
        sessionKey="oc_1"
        detail={{
          ...detail,
          turn_engaged: true,
          pending: [
            { id: 2, text: 'first', position: 0, disposition: 'staging', priority: false },
            { id: 3, text: 'second', position: 1, disposition: 'staging', priority: false },
          ],
        }}
        reachability={{ ok: true }}
        onRefresh={() => {}}
        movePending={move}
      />,
    )
    expect(screen.getByText('first')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '下移提交 1' }))
    expect(move).toHaveBeenCalledWith('oc_1', 2, 1)
  })
})

describe('TranscriptView', () => {
  it('sanitizes untrusted markdown and keeps code blocks', () => {
    const { container } = render(
      <TranscriptView
        entries={[
          entry({ position: 0, kind: 'prompt', element_type: 'markdown', content: '问 <b>好</b>' }),
          entry({
            position: 1,
            content: '<img src=x onerror="window.__pwned=1"><script>window.__pwned=2</script>```js\nconst x = 1\n```',
          }),
        ]}
      />,
    )
    expect(container.querySelector('script')).toBeNull()
    expect(container.querySelector('img')?.getAttribute('onerror')).toBeNull()
    expect(container.querySelector('pre code')).not.toBeNull()
    expect((window as unknown as { __pwned?: number }).__pwned).toBeUndefined()
  })

  it('keeps a tool fold open across regrouping', () => {
    const base = [
      entry({ position: 0, kind: 'prompt', element_type: 'markdown', content: 'go' }),
      entry({ position: 1, element_type: 'thinking', content: 'thinking...' }),
      entry({ position: 2, element_type: 'tool', content: 'tool args', title: 'read · a.rs' }),
    ]
    const { container, rerender } = render(<TranscriptView entries={base} />)
    const details = container.querySelector('details')!
    expect(details.open).toBe(false)
    fireEvent.click(details.querySelector('summary')!)
    expect(details.open).toBe(true)

    rerender(
      <TranscriptView
        entries={[...base, entry({ position: 3, kind: 'content', element_type: 'markdown', content: 'result' })]}
      />,
    )
    expect(container.querySelector('details')!.open).toBe(true)
  })

  it('renders the unread seam before the first unread segment', () => {
    render(
      <TranscriptView
        seamAnchor={1}
        entries={[
          entry({ position: 0, kind: 'prompt', element_type: 'markdown', content: 'old' }),
          entry({ position: 1, content: 'read reply' }),
          entry({ position: 2, kind: 'prompt', element_type: 'markdown', content: 'again' }),
          entry({ position: 3, content: 'new reply' }),
        ]}
      />,
    )
    const seam = screen.getByRole('separator', { name: /未读分界/ })
    expect(seam).toBeTruthy()
    expect(seam.getAttribute('aria-label')).toContain('1 段未读')
  })

  it('renders the escalation downgrade as its own labelled entry', () => {
    render(
      <TranscriptView
        entries={[
          entry({ position: 0, kind: 'prompt', element_type: 'markdown', content: 'perm' }),
          entry({
            position: 1,
            element_type: 'escalate_downgrade',
            content: JSON.stringify({
              request_id: 'claude:tc-1',
              tool: 'Bash',
              reason: 'need sudo once',
              detail: '该决策已按「仅放行一次」降级执行',
            }),
          }),
        ]}
      />,
    )
    const row = screen.getByTestId('escalate-downgrade-entry')
    expect(row.textContent).toContain('审批降级')
    expect(row.textContent).toContain('仅放行一次')
    expect(row.textContent).toContain('Bash')
    expect(row.textContent).toContain('need sudo once')
    // Never rendered as an anonymous 系统 line.
    expect(screen.queryByText('系统')).toBeNull()
  })

  it('keeps a malformed downgrade payload readable instead of throwing', () => {
    render(
      <TranscriptView
        entries={[entry({ position: 0, element_type: 'escalate_downgrade', content: 'not json' })]}
      />,
    )
    const row = screen.getByTestId('escalate-downgrade-entry')
    expect(row.textContent).toContain('审批降级')
    expect(row.textContent).toContain('not json')
  })
})
