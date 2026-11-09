/**
 * Framework-agnostic transcript model — pure-function coverage.
 *
 * Migrated verbatim from the retired Lit view suite (`transcript-view.test.ts`)
 * when the React workbench became the only production entry: every assertion
 * here targets `views/transcript-model.ts`, which React imports directly.
 * Rendered-markup assertions from the old suite are covered by the React
 * component tests and the Playwright journeys instead.
 */

import { describe, expect, it } from 'vitest'
import type { ConversationEntryView } from '../api/client.js'
import {
  ERROR_MERGE_WINDOW_SECS,
  TRUNCATE_CHARS,
  TRUNCATE_LINES,
  awaitingReceipt,
  closeUnclosedFence,
  deniedDetailContent,
  deniedLabel,
  entriesAwaitReceipt,
  errorEntryLabel,
  foldCrossRunToolResults,
  groupConversation,
  isDecidedToolResult,
  mergeAdjacentThinking,
  mergeSpawnErrors,
  mergeToolCalls,
  middleTruncate,
  processItemLabel,
  processRunDenied,
  processRunKind,
  processRunSummary,
  resolveAgentDisplay,
  splitAgentRuns,
  toolCallBlockDenied,
  toolCallBlockOutcome,
  toolResultDenied,
  truncateHtml,
  turnObservedModels,
  unitMaxTs,
  unitSegmentCount,
  type ProcessItem,
  type ProcessRun,
  type ToolCallBlock,
} from './transcript-model.js'

const FIXED_DATES: Record<string, number> = {
  // 2025-01-01T12:00:00Z, 2025-01-01T12:00:01Z, ...
  T1: 1735732800,
  T2: 1735732801,
  T3: 1735732802,
  T4: 1735732803,
  T5: 1735732804,
}

function entry(e: Partial<ConversationEntryView>): ConversationEntryView {
  return {
    position: 0,
    kind: 'content',
    element_type: 'markdown',
    content: '',
    created_at_unix: 0,
    ...e,
  }
}

/** A streamed agent turn: prompt + N text chunks (the wire's chunk level). */
function streamedTurn(prompt: string, chunks: string[], startAt: number): ConversationEntryView[] {
  const out: ConversationEntryView[] = [
    entry({ position: 0, kind: 'prompt', content: prompt, created_at_unix: startAt }),
  ]
  chunks.forEach((c, i) =>
    out.push(
      entry({
        position: i + 1,
        kind: 'content',
        content: c,
        created_at_unix: startAt,
      }),
    ),
  )
  return out
}

/** An agent turn mixing text and process entries around a prompt. */
function mixedTurnEntries(): ConversationEntryView[] {
  return [
    entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
    entry({ position: 1, kind: 'content', element_type: 'thinking', content: 'plan', created_at_unix: FIXED_DATES.T1 }),
    entry({ position: 2, kind: 'content', content: 'step one.', created_at_unix: FIXED_DATES.T1 }),
    entry({ position: 3, kind: 'content', element_type: 'tool', content: '📖 **read**', created_at_unix: FIXED_DATES.T1 }),
    entry({ position: 4, kind: 'content', content: 'mid text.', created_at_unix: FIXED_DATES.T2 }),
    entry({ position: 5, kind: 'content', element_type: 'thinking', content: 'reconsider', created_at_unix: FIXED_DATES.T2 }),
    entry({ position: 6, kind: 'content', element_type: 'tool', content: '✓ **bash**', created_at_unix: FIXED_DATES.T2 }),
    entry({ position: 7, kind: 'content', content: 'final.', created_at_unix: FIXED_DATES.T3 }),
  ]
}

describe('splitAgentRuns (D1, 1.1)', () => {
  it('merges contiguous same-kind entries: text concatenates, process accumulates', () => {
    const runs = splitAgentRuns([
      entry({ position: 1, content: 'a' }),
      entry({ position: 2, content: 'b' }),
      entry({ position: 3, element_type: 'tool', content: '📖 **read**' }),
      entry({ position: 4, element_type: 'tool', content: '✓ **read**' }),
      entry({ position: 5, content: 'c' }),
    ])
    expect(runs.map((r) => r.type)).toEqual(['text', 'process', 'text'])
    const [t1, proc, t2] = runs
    if (t1.type !== 'text' || proc.type !== 'process' || t2.type !== 'text') {
      return expect.unreachable()
    }
    expect(t1.content).toBe('ab')
    expect(t1.position).toBe(1)
    expect(proc.items.map((it) => it.position)).toEqual([3, 4])
    expect(proc.position).toBe(3)
    expect(t2.content).toBe('c')
    expect(t2.position).toBe(5)
  })

  it('alternates at every kind change — 正文-过程-正文交错', () => {
    const runs = splitAgentRuns(mixedTurnEntries().filter((e) => e.kind === 'content'))
    expect(runs.map((r) => r.type)).toEqual(['process', 'text', 'process', 'text', 'process', 'text'])
    const procs = runs.flatMap((r) => (r.type === 'process' ? [r] : []))
    expect(procs.map((r) => r.position)).toEqual([1, 3, 5])
    expect(procs[2].items.map((it) => it.position)).toEqual([5, 6])
    expect(procs[2].items.map((it) => it.elementType)).toEqual(['thinking', 'tool'])
  })

  it('thinking and tool are the same kind — one run, both inside', () => {
    const runs = splitAgentRuns([
      entry({ position: 1, element_type: 'thinking', content: 'hmm' }),
      entry({ position: 2, element_type: 'tool', content: '📖 **read**' }),
    ])
    expect(runs).toHaveLength(1)
    if (runs[0].type !== 'process') return expect.unreachable()
    expect(runs[0].items.map((it) => it.elementType)).toEqual(['thinking', 'tool'])
  })

  it('single entries keep their run shape', () => {
    expect(splitAgentRuns([entry({ position: 1, content: 'only' })].map((e) => e))).toEqual([
      { type: 'text', content: 'only', position: 1 },
    ])
    const proc = splitAgentRuns([
      entry({ position: 1, element_type: 'tool', content: 'x', title: 't', tool_use_id: 'tc-1' }),
    ])
    if (proc[0].type !== 'process') return expect.unreachable()
    // fold-tool-calls-into-process-tree 4.1：条目携带上游 call id（配对键）。
    expect(proc[0].items[0]).toEqual({
      elementType: 'tool',
      content: 'x',
      title: 't',
      toolUseId: 'tc-1',
      position: 1,
    } satisfies ProcessItem)
  })
})

describe('groupConversation (run model)', () => {
  it('one agent turn from N chunks — turn grouping, not entry grouping', () => {
    const units = groupConversation(
      mergeSpawnErrors(streamedTurn('do it', ['a', 'b', 'c', 'd'], FIXED_DATES.T1)),
    )
    expect(units).toHaveLength(2)
    expect(units[0].kind).toBe('operator')
    expect(units[1].kind).toBe('agent')
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    const text = agent.runs[0]
    expect(text.type).toBe('text')
    if (text.type !== 'text') return expect.unreachable()
    expect(text.content).toBe('abcd')
  })

  // ── fix-webui-qa-round10 2.4（B-DEF-02）：回合观察模型时间线（纯函数）──

  it('turnObservedModels replays model_change timeline per agent turn (round10 2.4)', async () => {
    const modelChange = (pos: number, from: string | null, to: string) =>
      entry({
        position: pos,
        kind: 'content',
        element_type: 'model_change',
        content: JSON.stringify({ from, to }),
        created_at_unix: FIXED_DATES.T1,
      })
    const units = groupConversation([
      entry({ position: 0, kind: 'prompt', content: 'q1', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'a1', created_at_unix: FIXED_DATES.T1 }),
      modelChange(2, null, 'b'),
      entry({ position: 3, kind: 'prompt', content: 'q2', created_at_unix: FIXED_DATES.T2 }),
      entry({ position: 4, kind: 'content', content: 'a2', created_at_unix: FIXED_DATES.T2 }),
    ])
    const timeline = turnObservedModels(units)
    // 切换前的回合无观察值（空串 = 无徽章）；切换后的回合取新模型。
    expect(timeline.get(1)).toBe('')
    expect(timeline.get(4)).toBe('b')
    // model_change 自身不产生映射。
    expect(timeline.size).toBe(2)
  })

  it('turnObservedModels: a mid-turn switch splits the turn and the tail takes the new model', async () => {
    const units = groupConversation([
      entry({ position: 0, kind: 'prompt', content: 'q', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'before ', created_at_unix: FIXED_DATES.T1 }),
      entry({
        position: 2,
        kind: 'content',
        element_type: 'model_change',
        content: JSON.stringify({ from: 'a', to: 'b' }),
        created_at_unix: FIXED_DATES.T1,
      }),
      entry({ position: 3, kind: 'content', content: 'after', created_at_unix: FIXED_DATES.T2 }),
    ])
    // model_change 终结当前 agent 单元：切换后的条目自成新单元，取新模型
    // （「切换从下一回合生效」语义）。
    const timeline = turnObservedModels(units)
    expect(timeline.get(1)).toBe('')
    expect(timeline.get(3)).toBe('b')
  })

  it('text → tool → text: the process run sits between the text segments at its arrival position (2.1)', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'let me check.', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'tool', content: '📖 **read**', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', element_type: 'tool', content: '✓ **read**', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 4, kind: 'content', content: 'done.', created_at_unix: FIXED_DATES.T2 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units).toHaveLength(2)
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    expect(agent.runs.map((r) => r.type)).toEqual(['text', 'process', 'text'])
    const proc = agent.runs[1]
    if (proc.type !== 'process') return expect.unreachable()
    expect(proc.position).toBe(2)
    expect(proc.items.map((it) => it.position)).toEqual([2, 3])
  })

  it('a multi-run turn alternates runs at their arrival positions (2.1)', () => {
    const units = groupConversation(mergeSpawnErrors(mixedTurnEntries()))
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    // 时间序：每个过程 run 折叠在自己的发生位置，不再是回合级单折叠。
    expect(agent.runs.map((r) => r.type)).toEqual([
      'process',
      'text',
      'process',
      'text',
      'process',
      'text',
    ])
    const procs = agent.runs.flatMap((r) => (r.type === 'process' ? [r] : []))
    expect(procs.map((r) => r.position)).toEqual([1, 3, 5])
    expect(procs[0].items.map((it) => it.position)).toEqual([1])
    expect(procs[1].items.map((it) => it.position)).toEqual([3])
    expect(procs[2].items.map((it) => it.position)).toEqual([5, 6])
    const texts = agent.runs.flatMap((r) => (r.type === 'text' ? [r.content] : []))
    expect(texts).toEqual(['step one.', 'mid text.', 'final.'])
  })

  it('a pure-text turn produces no process fold (2.1)', () => {
    const units = groupConversation(
      mergeSpawnErrors(streamedTurn('hi', ['a', 'b', 'c'], FIXED_DATES.T1)),
    )
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    expect(agent.runs.map((r) => r.type)).toEqual(['text'])
  })

  it('carries the entry title into process items; missing title tolerates legacy entries (2.2)', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'tool', content: '📖 **read**', title: 'read · src/main.rs', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'thinking', content: 'hmm', created_at_unix: FIXED_DATES.T1 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    const agent = units[1]
    if (agent.kind !== 'agent') return expect.unreachable()
    const proc = agent.runs[0]
    if (proc.type !== 'process') return expect.unreachable()
    expect(proc.items[0].title).toBe('read · src/main.rs')
    expect(proc.items[1].title).toBeNull()
  })

  it('conversation sides alternate in transcript order', () => {
    const entries = [
      ...streamedTurn('first', ['hello'], FIXED_DATES.T1),
      ...streamedTurn('second', ['world'], FIXED_DATES.T3).map((e, i) => ({ ...e, position: 2 + i })),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => u.kind)).toEqual(['operator', 'agent', 'operator', 'agent'])
  })

  it('error entries stay standalone units (fail-fast 3.3)', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', element_type: 'error', content: '**spawn failed**: x', created_at_unix: FIXED_DATES.T1 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => u.kind)).toEqual(['operator', 'error'])
  })

  it('an error mid-turn splits the agent turn and never joins a run (D5, 1.1)', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'before.', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 2, kind: 'content', element_type: 'error', content: '**spawn failed**: x', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 3, kind: 'content', content: 'after.', created_at_unix: FIXED_DATES.T2 }),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => u.kind)).toEqual(['operator', 'agent', 'error', 'agent'])
    for (const u of units) {
      if (u.kind !== 'agent') continue
      expect(u.runs.map((r) => r.type)).toEqual(['text'])
    }
  })
})

describe('process run ids (D2, 1.2)', () => {
  it('the run id is the first entry position and stays stable across regrouping', () => {
    const base = mixedTurnEntries()
    const ids = (entries: ConversationEntryView[]): number[] =>
      groupConversation(mergeSpawnErrors(entries)).flatMap((u) =>
        u.kind === 'agent'
          ? u.runs.flatMap((r) => (r.type === 'process' ? [r.position] : []))
          : [],
      )
    const before = ids(base)
    expect(before).toEqual([1, 3, 5])
    // 流式追加（尾部文本段后新开一个过程 run）：既有 run 的 id 全部原位
    // 不动，新 run 拿自己的首条目 position——折叠展开状态据此跨重渲染恢复。
    const after = ids([
      ...base,
      entry({ position: 8, kind: 'content', element_type: 'tool', content: '✓ **edit**', created_at_unix: FIXED_DATES.T3 }),
      entry({ position: 9, kind: 'content', content: 'tail.', created_at_unix: FIXED_DATES.T3 }),
    ])
    expect(after).toEqual([1, 3, 5, 8])
  })

  it('run id always equals its first item position', () => {
    const runs = groupConversation(
      mergeSpawnErrors(mixedTurnEntries()),
    ).flatMap((u) => (u.kind === 'agent' ? u.runs : []))
    for (const r of runs) {
      if (r.type !== 'process') continue
      expect(r.position).toBe(r.items[0]?.position)
    }
  })
})

describe('processRunSummary (D3)', () => {
  const run = (items: ProcessItem[]): ProcessRun => ({ type: 'process', items, position: items[0]?.position ?? 0 })

  it('reflects the running (last) entry: structured title wins', () => {
    const s = processRunSummary(
      run([
        { elementType: 'thinking', content: 'hmm', title: null, position: 1 },
        { elementType: 'tool', content: '📖 **read**', title: 'read · src/main.rs', position: 2 },
      ]),
    )
    expect(s.label).toBe('read · src/main.rs')
    expect(s.full).toBe('read · src/main.rs')
  })

  it('falls back to the generic element-type label for untitled entries', () => {
    const s = processRunSummary(
      run([{ elementType: 'tool', content: '✓ **bash**', title: null, position: 3 }]),
    )
    expect(s.label).toBe('tool')
    expect(s.full).toBeNull()
  })

  it('as entries stream in, the moving tail changes the summary', () => {
    const r = run([{ elementType: 'thinking', content: 'hmm', title: null, position: 1 }])
    expect(processRunSummary(r).label).toBe('thinking')
    r.items.push({ elementType: 'tool', content: 'x', title: 'write · a.rs', position: 2 })
    expect(processRunSummary(r).label).toBe('write · a.rs')
  })
})

describe('process honesty for denied tool results (polish-workbench-walkthrough-ux 5.6)', () => {
  const run = (items: ProcessItem[]): ProcessRun => ({ type: 'process', items, position: items[0]?.position ?? 0 })

  it('toolResultDenied: 明确拒绝记号判定（denied / 已拒绝 / ❌），不猜工具语义', () => {
    expect(toolResultDenied('Bash command rejected by user', 'bash')).toBe(true)
    expect(toolResultDenied('已拒绝执行 rm -rf /', null)).toBe(true)
    expect(toolResultDenied('❌ **bash**', 'bash')).toBe(true)
    // 正常成功结果不带拒绝记号。
    expect(toolResultDenied('✓ **bash**', 'bash')).toBe(false)
    expect(toolResultDenied('done in 1.2s', null)).toBe(false)
    // title 里的拒绝记号同样命中。
    expect(toolResultDenied('whatever', 'read · denied')).toBe(true)
  })

  it('deniedLabel: 去掉成功 ✓ 前缀，改 ✗', () => {
    expect(deniedLabel('✓ bash')).toBe('✗ bash')
    expect(deniedLabel('✓ **bash**')).toBe('✗ **bash**')
    // 无 ✓ 前缀的标签原样挂 ✗。
    expect(deniedLabel('bash')).toBe('✗ bash')
  })

  it('processItemLabel: 拒绝条目不再挂 ✓', () => {
    const denied = processItemLabel({
      elementType: 'tool',
      content: 'denied by operator',
      title: '✓ bash',
      position: 1,
    })
    expect(denied.label).toBe('✗ bash')
    expect(denied.full).toBe('bash')
    const ok = processItemLabel({
      elementType: 'tool',
      content: '✓ **bash**',
      title: 'bash · build',
      position: 2,
    })
    expect(ok.label).toBe('bash · build')
  })

  it('processRunDenied: run 内任一条目被拒即整行不显示成功语义', () => {
    const r = run([
      { elementType: 'tool', content: 'ok', title: 'bash · one', position: 1 },
      { elementType: 'tool', content: '已拒绝', title: 'bash · two', position: 2 },
    ])
    expect(processRunDenied(r)).toBe(true)
    expect(
      processRunDenied(run([{ elementType: 'tool', content: '✓ **bash**', title: null, position: 1 }])),
    ).toBe(false)
  })
})

describe('mergeSpawnErrors', () => {
  const SPAWN_ERR = '**spawn failed**: agent binary missing'
  const errEntry = (at: number, content = SPAWN_ERR): ConversationEntryView =>
    entry({ kind: 'content', element_type: 'error', content, created_at_unix: at })

  it('merges adjacent identical errors within the window into one counted entry', () => {
    const merged = mergeSpawnErrors([errEntry(100, SPAWN_ERR), errEntry(100 + ERROR_MERGE_WINDOW_SECS - 1, SPAWN_ERR)])
    expect(merged).toHaveLength(1)
    expect(merged[0].count).toBe(2)
    expect(merged[0].created_at_unix).toBe(100)
  })

  it('keeps identical errors outside the window as separate entries', () => {
    const split = mergeSpawnErrors([errEntry(100), errEntry(100 + ERROR_MERGE_WINDOW_SECS + 1)])
    expect(split).toHaveLength(2)
    expect(split.every((e) => e.count === 1)).toBe(true)
  })

  it('does not merge errors with different reasons; non-errors reset adjacency', () => {
    const A = 'err-a'
    const split = mergeSpawnErrors([errEntry(100, A), errEntry(101, 'other reason')])
    expect(split).toHaveLength(2)
    const reset = mergeSpawnErrors([
      errEntry(100, A),
      entry({ content: 'x', created_at_unix: 101 }),
      errEntry(102, A),
    ])
    expect(reset).toHaveLength(3)
    expect(reset.filter((e) => e.element_type === 'error').every((e) => e.count === 1)).toBe(true)
  })
})

// ---- middle truncation (2.3) ---------------------------------------------

describe('middleTruncate (2.3)', () => {
  const graphemeCount = (s: string): number => {
    const seg = new Intl.Segmenter('en', { granularity: 'grapheme' })
    return [...seg.segment(s)].length
  }

  it('keeps short strings (and strings at the 64-char boundary) as-is', () => {
    expect(middleTruncate('read · src/main.rs')).toBe('read · src/main.rs')
    expect(middleTruncate('a'.repeat(64))).toBe('a'.repeat(64))
  })

  it('collapses the middle of long strings into head 28 + … + tail 28', () => {
    expect(middleTruncate('a'.repeat(65))).toBe('a'.repeat(28) + '…' + 'a'.repeat(28))
    const t = middleTruncate('x'.repeat(200))
    expect(graphemeCount(t)).toBe(28 + 1 + 28)
    expect(t).toContain('…')
  })

  it('never splits multi-byte characters — Chinese, ZWJ family emoji, flags', () => {
    // 中文：按字（码点）切，绝无半个字符的替换符。
    const han = '汉字测试'.repeat(20) // 80 chars > 64
    const t1 = middleTruncate(han)
    expect(t1).not.toContain('\uFFFD')
    expect(t1.startsWith('汉字测试')).toBe(true)
    expect(t1.endsWith('汉字测试')).toBe(true)
    expect(graphemeCount(t1)).toBe(57)
    // ZWJ 家庭 emoji：一个字素 = 多个码点，截断不得把它切成两半。
    const family = '👨‍👩‍👧‍👦'
    const t2 = middleTruncate(family.repeat(70))
    expect(graphemeCount(t2)).toBe(57)
    expect(t2).toContain(family)
    expect(t2.startsWith(family)).toBe(true)
    // 旗标：两个区域指示符为一个字素。
    const flag = '🇨🇳'
    const t3 = middleTruncate(flag.repeat(70))
    expect(graphemeCount(t3)).toBe(57)
    expect(t3).toContain(flag)
  })
})

describe('assistant author label fallback chain (3.1, D1)', () => {
  it('resolves display → slug → assistant in order', () => {
    expect(resolveAgentDisplay('Claude Code')).toBe('Claude Code')
    expect(resolveAgentDisplay('claude-code')).toBe('claude-code')
    expect(resolveAgentDisplay(null)).toBe('assistant')
    expect(resolveAgentDisplay(undefined)).toBe('assistant')
    expect(resolveAgentDisplay('   ')).toBe('assistant')
  })

  it('derives the badge purely from the entry sequence, no Working gate (3.2)', () => {
    const promptOnly = groupConversation(
      mergeSpawnErrors(streamedTurn('q', [], FIXED_DATES.T1)),
    )
    // 排队窗：首个流式事件前会话显示 queued（非 working），角标仍须出现。
    expect(awaitingReceipt(promptOnly)).toBe(true)
    const withReply = groupConversation(
      mergeSpawnErrors(streamedTurn('q', ['answer'], FIXED_DATES.T1)),
    )
    // agent entry 一到（会话此时才翻 working）角标即消失。
    expect(awaitingReceipt(withReply)).toBe(false)
  })
})

describe('truncateHtml (4.5，纯函数)', () => {
  it('passes short content through untouched', () => {
    const r = truncateHtml({ content: 'short\nanswer' })
    expect(r.truncated).toBe(false)
    expect(r.preview).toBe('short\nanswer')
    expect(r.omittedLines).toBe(0)
    expect(r.omittedChars).toBe(0)
  })

  it('truncates by the line threshold and reports omitted lines/chars', () => {
    const lines = Array.from({ length: 60 }, (_, i) => `line-${i}`)
    const content = lines.join('\n')
    const r = truncateHtml({ content })
    expect(r.truncated).toBe(true)
    expect(r.preview).toBe(lines.slice(0, TRUNCATE_LINES).join('\n'))
    expect(r.omittedLines).toBe(60 - TRUNCATE_LINES)
    expect(r.omittedChars).toBe(content.length - r.preview.length)
  })

  it('truncates by the char threshold when it hits first', () => {
    const content = 'x'.repeat(TRUNCATE_CHARS + 500) // 单行：行阈值不触发
    const r = truncateHtml({ content })
    expect(r.truncated).toBe(true)
    expect(r.preview).toHaveLength(TRUNCATE_CHARS)
    expect(r.omittedChars).toBe(500)
    expect(r.omittedLines).toBe(0)
  })

  it('the earlier threshold wins when both exceed', () => {
    const content = Array.from({ length: 100 }, () => 'y'.repeat(200)).join('\n')
    const r = truncateHtml({ content })
    // 字符阈值（8_000）先到：行阈值切点是 40×200+39 = 8_039。
    expect(r.preview).toHaveLength(TRUNCATE_CHARS)
    expect(r.truncated).toBe(true)
  })
})

describe('errorEntryLabel (fix-webui-qa-defects 5.2, design D5)', () => {
  it('labels a spawn failure as spawn failed', () => {
    expect(errorEntryLabel({ failure_class: 'spawn' })).toBe('启动失败')
  })

  it('labels a stall force-settle as 回合停滞, never as a spawn failure', () => {
    const label = errorEntryLabel({ failure_class: 'stall' })
    expect(label).toContain('停滞')
    expect(label).not.toContain('spawn')
  })

  it('labels generic agent-turn errors (refusal included) neutrally', () => {
    expect(errorEntryLabel({ failure_class: 'generic' })).toBe('错误')
    expect(errorEntryLabel({ failure_class: 'generic', content: 'I cannot help with that.' })).toBe(
      '错误',
    )
  })

  it('falls back to the neutral label for legacy entries without a class', () => {
    expect(errorEntryLabel({})).toBe('错误')
    expect(errorEntryLabel({ failure_class: null })).toBe('错误')
    expect(errorEntryLabel({ failure_class: undefined })).toBe('错误')
  })
})

// ── fix-webui-qa-defects 5.3：被拒工具条目的展开详情与折叠标题一致（✗）────

describe('deniedDetailContent (fix-webui-qa-defects 5.3)', () => {
  const item = (content: string, title?: string | null): ProcessItem => ({
    elementType: 'tool',
    content,
    title,
    position: 0,
  })

  it('rewrites the leading approved prefix of a denied tool result to ✗', () => {
    const content = '✓ **Bash**\n已拒绝 by policy'
    const out = deniedDetailContent(content, item(content, '✓ Bash').title)
    expect(out.startsWith('✗')).toBe(true)
    expect(out.startsWith('✓')).toBe(false)
  })

  it('leaves a non-denied tool result untouched', () => {
    const content = '✓ **Read**\nfile contents'
    expect(deniedDetailContent(content, null)).toBe(content)
  })

  it('flags denial from the structured title even when the result text is plain', () => {
    const content = '✓ **Bash**\nexit 1'
    const out = deniedDetailContent(content, '✓ Bash · denied')
    expect(out.startsWith('✗')).toBe(true)
  })

  it('keeps content without a ✓ prefix intact (nothing to rewrite)', () => {
    const content = '已拒绝：普通文本结果（无 ✓ 前缀）'
    expect(deniedDetailContent(content, null)).toBe(content)
  })
})

describe('notice entries (close-acceptance-blind-spots 4.2, design D3)', () => {
  const noticeEntry = (position: number, ts: number = FIXED_DATES.T2): ConversationEntryView =>
    entry({
      position,
      kind: 'content',
      element_type: 'notice',
      content: '**回合已结束且无输出**：本轮回合未产生任何可见输出。',
      created_at_unix: ts,
    })

  it('notice entries stay standalone units that split the surrounding agent turn', () => {
    const entries = [
      entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }),
      entry({ position: 1, kind: 'content', content: 'partial', created_at_unix: FIXED_DATES.T1 }),
      noticeEntry(2),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => u.kind)).toEqual(['operator', 'agent', 'notice'])
    expect(unitMaxTs(units[2]!)).toBe(FIXED_DATES.T2)
  })

  it('notice contributes zero visible segments (zero-output turn must not inflate unread)', () => {
    const entries = [entry({ position: 0, kind: 'prompt', content: 'go', created_at_unix: FIXED_DATES.T1 }), noticeEntry(1)]
    const units = groupConversation(mergeSpawnErrors(entries))
    expect(units.map((u) => unitSegmentCount(u))).toEqual([0, 0])
  })
})

describe('tool call blocks merge invocation and result (fold-tool-calls 4.1)', () => {
  const requestEntry = (pos: number, id?: string): ConversationEntryView => ({
    position: pos,
    kind: 'content',
    element_type: 'tool',
    content: '📖 **Bash**\n```json\n{"command": "rm -rf /"}\n```',
    title: 'Bash · rm -rf /',
    tool_use_id: id ?? null,
    created_at_unix: FIXED_DATES.T1,
  })
  const resultEntry = (
    pos: number,
    id?: string,
    content = '✓ **Bash**\nperm done\n',
  ): ConversationEntryView => ({
    position: pos,
    kind: 'content',
    element_type: 'tool',
    content,
    title: '✓ Bash',
    tool_use_id: id ?? null,
    created_at_unix: FIXED_DATES.T1,
  })

  it('isDecidedToolResult: ✓/✗ titled tool entries are results; requests and thinking are not', () => {
    expect(isDecidedToolResult(resultEntry(0))).toBe(true)
    expect(isDecidedToolResult(resultEntry(0, undefined, '✗ **Bash**\ndenied by fake'))).toBe(true)
    expect(isDecidedToolResult(requestEntry(0))).toBe(false)
    expect(
      isDecidedToolResult({
        element_type: 'thinking',
        content: 'the request was denied earlier',
        title: null,
      }),
      // thinking entries are never results even when the text mentions denial.
    ).toBe(false)
  })

  it('splitAgentRuns: decided results stay inside the process run (no top-level tool_result runs)', () => {
    const runs = splitAgentRuns([requestEntry(0, 'tc-1'), resultEntry(1, 'tc-1')])
    expect(runs.map((r) => r.type)).toEqual(['process'])
    const fold = runs[0] as ProcessRun
    // 过程 run 收全部两条原始条目（合并是渲染层的纯派生）。
    expect(fold.items).toHaveLength(2)
  })

  it('mergeToolCalls: a paired invocation+result becomes ONE block', () => {
    const items = splitAgentRuns([requestEntry(0, 'tc-1'), resultEntry(1, 'tc-1')])
    if (items[0].type !== 'process') return expect.unreachable()
    const nodes = mergeToolCalls(items[0].items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.type).toBe('tool_call')
    expect(block.invocation.title).toBe('Bash · rm -rf /')
    expect(block.result!.title).toBe('✓ Bash')
    expect(block.position).toBe(0)
    // 配对成功 → 结果章 ✓（collapse 行即读）。
    expect(toolCallBlockOutcome(block)).toBe('ok')
    expect(toolCallBlockDenied(block)).toBe(false)
  })

  it('mergeToolCalls: an unpaired invocation renders as its own call-state block (no outcome)', () => {
    const items = splitAgentRuns([requestEntry(0, 'tc-1')])
    if (items[0].type !== 'process') return expect.unreachable()
    const nodes = mergeToolCalls(items[0].items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.type).toBe('tool_call')
    expect(block.result).toBeNull()
    // 调用态标题：无 ✓/✗ 章。
    expect(toolCallBlockOutcome(block)).toBeNull()
  })

  it('mergeToolCalls: concurrent calls to the same tool pair by id, never by position', () => {
    const entries = [
      requestEntry(0, 'tc-a'),
      { ...requestEntry(1, 'tc-b'), title: 'Read · /tmp/b', content: '📖 **Read**\n```json\n{"file_path": "/tmp/b"}\n```' },
      { ...resultEntry(2, 'tc-b'), content: '✓ **Read**\ncontent of b' },
      { ...resultEntry(3, 'tc-a'), content: '✓ **Read**\ncontent of a' },
    ]
    const runs = splitAgentRuns(entries)
    if (runs[0].type !== 'process') return expect.unreachable()
    const nodes = mergeToolCalls(runs[0].items)
    expect(nodes).toHaveLength(2)
    const [blockA, blockB] = nodes.map((n) => n as ToolCallBlock)
    // 结果乱序回流：各自按 id 回到自己的调用块——b 的结果先到也不串块。
    expect(blockA.invocation.title).toBe('Bash · rm -rf /')
    expect(blockA.result!.content).toContain('content of a')
    expect(blockB.invocation.title).toBe('Read · /tmp/b')
    expect(blockB.result!.content).toContain('content of b')
  })

  it('mergeToolCalls: a lone result (truncated transcript) still renders as its own block', () => {
    const runs = splitAgentRuns([resultEntry(0, 'tc-x')])
    if (runs[0].type !== 'process') return expect.unreachable()
    const nodes = mergeToolCalls(runs[0].items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.result).toBeNull()
    // 孤立 ✓ 条目自带已决标记 → 保留 ✓ 章（内容零丢失）。
    expect(toolCallBlockOutcome(block)).toBe('ok')
  })

  // ── fold-tool-calls-into-process-tree review F1：跨 run 配对 ──────────────
  // native 载体对泊车/决策落 markdown 条目（⏳/🛡），把同一调用的结果切进
  // 另一个过程 run；foldCrossRunToolResults 在回合范围内按 id 把结果搬回
  // 调用所在 run（spec「配对 SHALL NOT rely on arrival position」）。

  const textEntry = (pos: number, content: string): ConversationEntryView => ({
    position: pos,
    kind: 'content',
    element_type: 'markdown',
    content,
    created_at_unix: FIXED_DATES.T1,
  })

  it('foldCrossRunToolResults: a result split off by parking notes moves back to its invocation run', () => {
    // native 被门控序列：tool(调用) → markdown(⏳/🛡) → tool(✓ 结果)。
    const runs = splitAgentRuns([
      requestEntry(0, 'toolu_1'),
      textEntry(1, '⏳ bash awaits approval'),
      textEntry(2, '🛡 policy allow'),
      resultEntry(3, 'toolu_1'),
    ])
    expect(runs.map((r) => r.type)).toEqual(['process', 'text', 'process'])
    const regrouped = foldCrossRunToolResults(runs)
    // 结果搬回调用所在 run；被搬空的过程 run 丢弃；正文 run 原样保留。
    expect(regrouped.map((r) => r.type)).toEqual(['process', 'text'])
    const fold = regrouped[0] as ProcessRun
    expect(fold.items).toHaveLength(2)
    expect((regrouped[1] as { content: string }).content).toContain('awaits approval')
    // 逐 run 合并后：一块、带结果、✓ 章（调用块与汇总行都能读到达成）。
    const nodes = mergeToolCalls(fold.items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.result).not.toBeNull()
    expect(toolCallBlockOutcome(block)).toBe('ok')
  })

  it('foldCrossRunToolResults: parallel results scattered across runs each return to their own call', () => {
    const runs = splitAgentRuns([
      requestEntry(0, 'tc-a'),
      { ...requestEntry(1, 'tc-b'), title: 'Read · /tmp/b', content: '📖 **Read**' },
      textEntry(2, '⏳ a awaits approval'),
      textEntry(3, '⏳ b awaits approval'),
      { ...resultEntry(4, 'tc-b'), content: '✓ **Read**\nb done' },
      { ...resultEntry(5, 'tc-a'), content: '✓ **Bash**\na done' },
    ])
    const regrouped = foldCrossRunToolResults(runs)
    // 两个结果都回到第一个 run；中间泊车正文不动；收尾 run 搬空即弃。
    expect(regrouped.map((r) => r.type)).toEqual(['process', 'text'])
    const nodes = mergeToolCalls((regrouped[0] as ProcessRun).items)
    expect(nodes).toHaveLength(2)
    const [a, b] = nodes.map((n) => n as ToolCallBlock)
    expect(a.result!.content).toContain('a done')
    expect(b.result!.content).toContain('b done')
  })

  it('foldCrossRunToolResults: a lone result in a later run without any invocation stays put', () => {
    const runs = splitAgentRuns([resultEntry(0, 'tc-x'), textEntry(1, 'done'), resultEntry(2, 'tc-y')])
    const regrouped = foldCrossRunToolResults(runs)
    // 没有可认领的调用：三条 run 原样（孤立结果自成一块的契约不破坏）。
    expect(regrouped.map((r) => r.type)).toEqual(['process', 'text', 'process'])
    expect(regrouped).toBe(runs)
  })

  it('foldCrossRunToolResults: duplicate results — only the first is claimed, the rest stay', () => {
    const runs = splitAgentRuns([
      requestEntry(0, 'tc-1'),
      textEntry(1, '🛡 policy'),
      resultEntry(2, 'tc-1'),
      resultEntry(3, 'tc-1'),
    ])
    const regrouped = foldCrossRunToolResults(runs)
    expect(regrouped.map((r) => r.type)).toEqual(['process', 'text', 'process'])
    const first = mergeToolCalls((regrouped[0] as ProcessRun).items)
    expect(first).toHaveLength(1)
    expect((first[0] as ToolCallBlock).result).not.toBeNull()
    // 重复结果留在原地，走「未配对结果自成一块」的既有契约。
    const last = mergeToolCalls((regrouped[2] as ProcessRun).items)
    expect(last).toHaveLength(1)
    expect((last[0] as ToolCallBlock).result).toBeNull()
  })

  it('groupConversation: gated native turn renders ONE merged block in the invocation fold', () => {
    const entries: ConversationEntryView[] = [
      { position: 0, kind: 'prompt', element_type: 'markdown', content: 'go', created_at_unix: FIXED_DATES.T1 },
      requestEntry(1, 'toolu_1'),
      textEntry(2, '⏳ bash awaits approval'),
      resultEntry(3, 'toolu_1'),
    ]
    const units = groupConversation(mergeSpawnErrors(entries))
    const agent = units.find((u) => u.kind === 'agent')
    if (!agent || agent.kind !== 'agent') return expect.unreachable()
    const processRuns = agent.runs.filter((r) => r.type === 'process') as ProcessRun[]
    // 单棵树：整个回合只剩调用所在的一个过程 run。
    expect(processRuns).toHaveLength(1)
    const nodes = mergeToolCalls(processRuns[0].items)
    expect(nodes).toHaveLength(1)
    const block = nodes[0] as ToolCallBlock
    expect(block.type).toBe('tool_call')
    expect(block.result).not.toBeNull()
    expect(toolCallBlockOutcome(block)).toBe('ok')
  })
})

describe('entriesAwaitReceipt ignores empty prompt entries (round2 1.2)', () => {
  it('an empty-content prompt tail is not a submission', () => {
    expect(
      entriesAwaitReceipt([
        { kind: 'content', content: 'hi' },
        { kind: 'prompt', content: '' },
      ]),
    ).toBe(false)
    expect(
      entriesAwaitReceipt([
        { kind: 'content', content: 'hi' },
        { kind: 'prompt', content: '/compact' },
      ]),
    ).toBe(true)
    // content 字段缺席（旧调用方形状）按非空对待，语义不变。
    expect(entriesAwaitReceipt([{ kind: 'prompt' }])).toBe(true)
    expect(entriesAwaitReceipt([{ kind: 'content', content: 'x' }])).toBe(false)
  })
})

describe('round8 pure helpers (fence tolerance, thinking merge)', () => {
  it('closeUnclosedFence closes an unbalanced fence and leaves balanced content alone (7.2)', async () => {
    const unclosed = 'code:\n```js\nlet x = 1'
    expect(closeUnclosedFence(unclosed)).toBe(unclosed + '\n```')
    expect(closeUnclosedFence('```js\nok\n```')).toBe('```js\nok\n```')
    expect(closeUnclosedFence('plain text')).toBe('plain text')
    // 成对围栏夹单行文本：仍配平。
    expect(closeUnclosedFence('```a\n```\nmid\n```b\n```')).toBe('```a\n```\nmid\n```b\n```')
  })

  it('mergeAdjacentThinking merges adjacent thinking items and keeps tool boundaries (7.1)', async () => {
    const items: ProcessItem[] = [
      { elementType: 'thinking', content: 'a', title: null, position: 1 },
      { elementType: 'thinking', content: 'b', title: null, position: 2 },
      { elementType: 'tool', content: 'x', title: null, position: 3 },
      { elementType: 'thinking', content: 'c', title: null, position: 4 },
    ]
    const merged = mergeAdjacentThinking(items)
    expect(merged).toHaveLength(3)
    expect(merged[0]).toMatchObject({ elementType: 'thinking', content: 'ab', position: 1 })
    expect(merged[1]?.elementType).toBe('tool')
    expect(merged[2]).toMatchObject({ elementType: 'thinking', content: 'c', position: 4 })
  })
})

describe('process run kinds (round3 D1)', () => {
  it('splitAgentRuns never routes text entries into process runs (alternating turn)', () => {
    const runs = splitAgentRuns([
      entry({ position: 1, element_type: 'thinking', content: 'hmm' }),
      entry({ position: 2, element_type: 'markdown', content: 'part one. ' }),
      entry({ position: 3, element_type: 'thinking', content: 'hmm again' }),
      entry({ position: 4, element_type: 'markdown', content: 'part two.' }),
    ])
    expect(runs.map((r) => r.type)).toEqual(['process', 'text', 'process', 'text'])
    const texts = runs.filter((r): r is Extract<typeof r, { type: 'text' }> => r.type === 'text')
    expect(texts.map((t) => t.content)).toEqual(['part one. ', 'part two.'])
  })

  it('a tool-only fold keeps the tool kind; mixed runs are marked mixed', async () => {
    const runs: ProcessRun[] = [
      { type: 'process', position: 1, items: [{ elementType: 'tool', content: 'x', title: null, position: 1 }] },
      { type: 'process', position: 2, items: [
        { elementType: 'thinking', content: 'h', title: null, position: 2 },
        { elementType: 'tool', content: 't', title: null, position: 3 },
      ] },
    ]
    expect(processRunKind(runs[0]!)).toBe('tool')
    expect(processRunKind(runs[1]!)).toBe('mixed')
  })
})
