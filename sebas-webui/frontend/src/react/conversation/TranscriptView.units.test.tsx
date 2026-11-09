import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import type { ConversationElementType, ConversationEntryView } from '../../api/client.js'
import { ERROR_MERGE_WINDOW_SECS, TRUNCATE_LINES } from '../../views/transcript-model.js'
import { TranscriptView } from './TranscriptView.js'

afterEach(cleanup)

const T1 = 1_735_732_800

function entry(
  position: number,
  kind: 'prompt' | 'content',
  element_type: ConversationElementType,
  content: string,
  extra: Partial<ConversationEntryView> = {},
): ConversationEntryView {
  return {
    position,
    kind,
    element_type,
    content,
    created_at_unix: T1 + position,
    ...extra,
  }
}

describe('TranscriptView unit rendering', () => {
  it('renders both conversation sides with the operator receipt badge on the newest submission', () => {
    const view = render(
      <TranscriptView
        entries={[entry(0, 'prompt', 'markdown', 'question')]}
      />,
    )
    const receipt = view.container.querySelector('[data-receipt]')
    expect(receipt?.textContent).toBe('已收到')
    expect(receipt?.getAttribute('title')).toBe('服务端已接受，等待 agent 开始输出')
  })

  it('clears the receipt badge as soon as the agent reply arrives', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'prompt', 'markdown', 'question'),
          entry(1, 'content', 'markdown', 'answer'),
        ]}
      />,
    )
    expect(view.container.querySelector('[data-receipt]')).toBeNull()
  })

  it('badges only the newest prompt in an ongoing conversation', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'prompt', 'markdown', 'q1'),
          entry(1, 'content', 'markdown', 'a1'),
          entry(2, 'prompt', 'markdown', 'q2'),
        ]}
      />,
    )
    const badges = view.container.querySelectorAll('[data-receipt]')
    expect(badges).toHaveLength(1)
    expect(badges[0]!.closest('[data-unit-key]')?.textContent).toContain('q2')
  })

  it('labels a spawn failure by its failure_class, not a hardwired string', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'content', 'error', '**spawn failed**: missing binary', {
            failure_class: 'spawn',
          }),
        ]}
      />,
    )
    const alert = view.container.querySelector('[role="alert"]')
    // 标签行按 failure_class 渲染；正文是 agent 原文，保持不动。
    expect(alert?.firstElementChild?.textContent).toBe('启动失败')
  })

  it('labels a stall force-settle as a stall, never a spawn failure', () => {
    const view = render(
      <TranscriptView
        entries={[entry(0, 'content', 'error', 'turn stalled', { failure_class: 'stall' })]}
      />,
    )
    const alert = view.container.querySelector('[role="alert"]')
    expect(alert?.firstElementChild?.textContent).toBe('回合停滞')
  })

  it('merges adjacent identical errors into one counted bubble', () => {
    const mergedWindow = ERROR_MERGE_WINDOW_SECS - 1
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'content', 'error', 'spawn failed', { created_at_unix: 100 }),
          entry(1, 'content', 'error', 'spawn failed', { created_at_unix: 100 + mergedWindow }),
        ]}
      />,
    )
    const alerts = view.container.querySelectorAll('[role="alert"]')
    expect(alerts).toHaveLength(1)
    expect(alerts[0]!.textContent).toContain('×2')
  })

  it('shows the turn model chip only for turns with an observed model, never a placeholder', () => {
    const withModel = render(
      <TranscriptView
        entries={[
          entry(0, 'content', 'model_change', JSON.stringify({ from: null, to: 'test/model' })),
          entry(1, 'prompt', 'markdown', 'q'),
          entry(2, 'content', 'markdown', 'a'),
        ]}
      />,
    )
    expect(withModel.container.querySelector('[data-testid="turn-model"]')?.textContent).toBe(
      'test/model',
    )
    cleanup()
    const withoutModel = render(
      <TranscriptView
        entries={[entry(0, 'prompt', 'markdown', 'q'), entry(1, 'content', 'markdown', 'a')]}
      />,
    )
    expect(withoutModel.container.querySelector('[data-testid="turn-model"]')).toBeNull()
  })

  it('never rewrites an earlier turn badge after a later model switch', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'content', 'model_change', JSON.stringify({ from: null, to: 'first' })),
          entry(1, 'prompt', 'markdown', 'q1'),
          entry(2, 'content', 'markdown', 'a1'),
          entry(3, 'content', 'model_change', JSON.stringify({ from: 'first', to: 'second' })),
          entry(4, 'prompt', 'markdown', 'q2'),
          entry(5, 'content', 'markdown', 'a2'),
        ]}
      />,
    )
    const chips = [...view.container.querySelectorAll('[data-testid="turn-model"]')].map(
      (el) => el.textContent,
    )
    expect(chips).toEqual(['first', 'second'])
  })
})

describe('TranscriptView process folds', () => {
  const toolEntries = (): ConversationEntryView[] => [
    entry(0, 'prompt', 'markdown', 'perm'),
    entry(1, 'content', 'tool', '📖 **Bash**\n```json\n{"command": "rm -rf /"}\n```', {
      title: 'Bash · rm -rf /',
      tool_use_id: 'tc-1',
    }),
    entry(2, 'content', 'tool', '✓ **Bash**\nperm done', {
      title: '✓ Bash',
      tool_use_id: 'tc-1',
    }),
  ]

  it('renders one collapsed process fold per run, opened on demand', () => {
    const view = render(<TranscriptView entries={toolEntries()} />)
    const folds = view.container.querySelectorAll('details')
    expect(folds).toHaveLength(1)
    const fold = folds[0]!
    expect(fold.hasAttribute('open')).toBe(false)
    const summary = fold.querySelector('summary')!
    // 摘要跟踪 run 尾（结果条目）：结果是 `✓ Bash`。
    expect(summary.textContent).toContain('Bash')
    expect(summary.textContent).toContain('2 项')
    expect(summary.textContent).toContain('✓ 已执行')

    fireEvent.click(summary)
    expect(fold.hasAttribute('open')).toBe(true)
    expect(fold.textContent).toContain('rm -rf /')
    expect(fold.textContent).toContain('perm done')
    // 单层过程树：折叠体内没有嵌套的 details。
    expect(fold.querySelectorAll('details')).toHaveLength(0)
  })

  it('keeps a denied tool result marked ✗ while collapsed', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'content', 'tool', '📖 **Bash**', { title: 'Bash · x', tool_use_id: 'tc-9' }),
          entry(1, 'content', 'tool', '✓ **Bash**\ndenied by policy', {
            title: '✓ Bash',
            tool_use_id: 'tc-9',
          }),
        ]}
      />,
    )
    const fold = view.container.querySelector('details')!
    expect(fold.querySelector('[data-testid="tool-outcome-denied"]')?.textContent).toContain(
      '✗ 已拒绝',
    )
    expect(fold.querySelector('[data-testid="tool-outcome"]')).toBeNull()
    fireEvent.click(fold.querySelector('summary')!)
    // 展开体首行的 ✓ 同样改写为 ✗（折叠标题与详情口径一致）。
    const body = fold.textContent ?? ''
    expect(body).not.toContain('✓ **Bash**')
  })

  it('renders thinking content inline by default and merges adjacent increments', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'prompt', 'markdown', 'think please'),
          entry(1, 'content', 'thinking', 'hmm'),
          entry(2, 'content', 'thinking', 'hmm again'),
        ]}
      />,
    )
    const fold = view.container.querySelector('details')!
    fireEvent.click(fold.querySelector('summary')!)
    const items = fold.querySelectorAll('[data-element-type="thinking"]')
    expect(items).toHaveLength(1)
    expect(items[0]!.textContent).toContain('hmm')
    expect(items[0]!.textContent).toContain('hmm again')
  })

  it('renders thinking and text runs in their arrival order outside the folds', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'prompt', 'markdown', 'go'),
          entry(1, 'content', 'thinking', 'first thought'),
          entry(2, 'content', 'markdown', 'spoken one'),
          entry(3, 'content', 'thinking', 'second thought'),
          entry(4, 'content', 'markdown', 'spoken two'),
        ]}
      />,
    )
    const article = view.container.querySelector('article[class*="agent"]')!
    const kinds = [...article.children]
      .slice(1)
      .map((el) => (el.tagName.toLowerCase() === 'details' ? 'fold' : 'text'))
    expect(kinds).toEqual(['fold', 'text', 'fold', 'text'])
  })
})

describe('TranscriptView truncation + view-all', () => {
  const longContent = Array.from({ length: TRUNCATE_LINES + 10 }, (_, i) => `line-${i}`).join('\n')

  it('shows the preview, the omission notice and a view-all exit for over-threshold entries', () => {
    const view = render(
      <TranscriptView
        entries={[entry(0, 'content', 'thinking', longContent)]}
      />,
    )
    const fold = view.container.querySelector('details')!
    fireEvent.click(fold.querySelector('summary')!)
    const note = fold.querySelector('[data-testid="truncation-note"]')!
    expect(note).toBeTruthy()
    expect(note.textContent).toContain('已截断：省略 10 行')
    // 预览之外的行不在折叠体里（dom 只承载预览）。
    expect(fold.textContent).not.toContain('line-49')
    expect(fold.textContent).toContain('line-0')
  })

  it('renders under-threshold entries in full with no truncation note', () => {
    const view = render(<TranscriptView entries={[entry(0, 'content', 'thinking', 'short')]} />)
    const fold = view.container.querySelector('details')!
    fireEvent.click(fold.querySelector('summary')!)
    expect(fold.querySelector('[data-testid="truncation-note"]')).toBeNull()
    expect(fold.textContent).toContain('short')
  })

  it('opens the full content in an isolated dialog and unmounts it on close', () => {
    const view = render(<TranscriptView entries={[entry(0, 'content', 'thinking', longContent)]} />)
    const fold = view.container.querySelector('details')!
    fireEvent.click(fold.querySelector('summary')!)
    fireEvent.click(screen.getByText('查看全部'))

    const dialog = screen.getByRole('dialog')
    expect(dialog).toBeTruthy()
    expect(dialog.textContent).toContain('line-49')
    // 对话滚动面不承载弹层内容（AntD portal 到 body，隔离于转录容器之外）。
    expect(fold.textContent).not.toContain('line-49')

    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})

describe('TranscriptView contract entries', () => {
  it('renders escalate_downgrade as a first-class entry with the tool and the reason', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(
            0,
            'content',
            'escalate_downgrade',
            JSON.stringify({
              request_id: 'req-1',
              tool: 'Bash',
              reason: '需要 sudo 装依赖',
              detail: '该决策已按「仅放行一次」降级执行。',
            }),
          ),
        ]}
      />,
    )
    const unit = view.container.querySelector('[data-testid="escalate-downgrade-entry"]')!
    expect(unit.textContent).toContain('仅放行一次')
    expect(unit.textContent).toContain('Bash')
    expect(unit.textContent).toContain('需要 sudo 装依赖')
    expect(unit.getAttribute('role')).toBeNull()
  })

  it('renders model_change with both models', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'content', 'model_change', JSON.stringify({ from: 'test/text', to: 'test/long' })),
        ]}
      />,
    )
    const unit = view.container.querySelector('[data-testid="model-change-entry"]')!
    expect(unit.textContent).toContain('test/text')
    expect(unit.textContent).toContain('test/long')
  })

  it('renders a zero-output notice as a neutral bar, not an error bubble', () => {
    const view = render(
      <TranscriptView
        entries={[
          entry(0, 'prompt', 'markdown', 'say nothing'),
          entry(1, 'content', 'notice', '**回合已结束且无输出**'),
        ]}
      />,
    )
    expect(view.container.querySelector('[role="alert"]')).toBeNull()
    expect(view.container.textContent).toContain('提示')
    expect(view.container.textContent).toContain('回合已结束且无输出')
  })

  it('skips entries with empty content', () => {
    const view = render(
      <TranscriptView
        entries={[entry(0, 'content', 'markdown', ''), entry(1, 'content', 'markdown', 'shown')]}
      />,
    )
    expect(view.container.querySelectorAll('[data-unit-key]')).toHaveLength(1)
    expect(view.container.textContent).toContain('shown')
  })

  it('keeps the unread seam before the first unit past the anchor', () => {
    const view = render(
      <TranscriptView
        seamAnchor={0}
        entries={[
          entry(0, 'prompt', 'markdown', 'q1'),
          entry(1, 'content', 'markdown', 'a1'),
          entry(2, 'prompt', 'markdown', 'q2'),
        ]}
      />,
    )
    const seam = view.container.querySelector('[role="separator"]')!
    expect(seam).toBeTruthy()
    expect(seam.textContent).toContain('未读')
    // 操作者回合贡献 0 段：锚 0 之下它的累计未越过锚，seam 落在第一个
    // 「把累计推过锚」的单元（agent 回合）里，画在单元自身之前。
    const agentUnit = view.container.querySelector('[data-unit-key^="agent-"]')!
    expect(agentUnit.firstElementChild).toBe(seam)
    expect(seam.getAttribute('aria-label')).toContain('段未读')
    // 已读完的锚（段数打满）不画缝。
    cleanup()
    const fullyRead = render(
      <TranscriptView
        seamAnchor={99}
        entries={[entry(0, 'prompt', 'markdown', 'q1'), entry(1, 'content', 'markdown', 'a1')]}
      />,
    )
    expect(fullyRead.container.querySelector('[role="separator"]')).toBeNull()
  })
})
