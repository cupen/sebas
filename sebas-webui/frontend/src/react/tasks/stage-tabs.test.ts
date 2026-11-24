import { describe, expect, it } from 'vitest'
import { tabForProject, withProjectTab } from './stage-tabs.js'

describe('stage tabs', () => {
  it('defaults to the conversation view', () => {
    expect(tabForProject({}, 'p1')).toBe('conversation')
    expect(tabForProject({ p1: 'tasks' }, null)).toBe('conversation')
  })

  it('keeps each project on its own last-active tab', () => {
    let state = withProjectTab({}, 'p1', 'tasks')
    state = withProjectTab(state, 'p2', 'conversation')
    expect(tabForProject(state, 'p1')).toBe('tasks')
    expect(tabForProject(state, 'p2')).toBe('conversation')
    state = withProjectTab(state, 'p2', 'tasks')
    expect(tabForProject(state, 'p1')).toBe('tasks')
    expect(tabForProject(state, 'p2')).toBe('tasks')
  })

  it('is a no-op for a null project or an unchanged tab', () => {
    const state = { p1: 'tasks' as const }
    expect(withProjectTab(state, null, 'conversation')).toBe(state)
    expect(withProjectTab(state, 'p1', 'tasks')).toBe(state)
    expect(withProjectTab({}, 'p1', 'tasks')).toEqual({ p1: 'tasks' })
  })
})
