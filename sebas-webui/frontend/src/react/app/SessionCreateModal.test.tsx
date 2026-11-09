import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AgentKindInfo, Project } from '../../api/client.js'
import type { LoadedCatalog } from '../../api/model-catalog.js'
import {
  SessionCreateModal,
  createDraftError,
  createSessionPayload,
} from './SessionCreateModal.js'

afterEach(cleanup)

const projects: Project[] = [{ id: 'p1', name: 'alpha', path: '/work/alpha', added_at: 1 }]

const catalog: LoadedCatalog = {
  catalog: {
    pairs: [{ provider: 'openrouter', model: 'deepseek-chat' }],
    defaultProvider: null,
    defaultModel: null,
  },
  aliases: [],
  unavailable: false,
}

const agents: AgentKindInfo[] = [
  { id: 'codex', display: 'Codex', reachable: true },
  { id: 'broken', display: 'Broken', reachable: false, cause: 'spawn failed' },
] as AgentKindInfo[]

describe('session create payload', () => {
  it('requires project and agent', () => {
    expect(createDraftError({ projectId: null, agent: 'a', prompt: '', mode: null, model: null })).toBe(
      '请选择项目',
    )
    expect(createDraftError({ projectId: 'p', agent: null, prompt: '', mode: null, model: null })).toBe(
      '请选择 agent',
    )
  })

  it('keeps the existing agent/project/model/mode wire shape', () => {
    expect(
      createSessionPayload({
        projectId: 'p1',
        agent: 'codex',
        prompt: '  ',
        mode: 'allow',
        model: 'deepseek-chat',
      }),
    ).toEqual({
      agent: 'codex',
      projectId: 'p1',
      prompt: null,
      model: 'deepseek-chat',
      mode: 'allow',
    })
    expect(
      createSessionPayload({ projectId: 'p1', agent: 'codex', prompt: 'do it', mode: null, model: null }),
    ).toMatchObject({ prompt: 'do it', model: null, mode: null })
  })
})

describe('SessionCreateModal', () => {
  it('shows an inline error when the agent is missing', async () => {
    render(
      <SessionCreateModal
        open
        projectId="p1"
        projects={projects}
        onClose={() => {}}
        onCreated={() => {}}
        loadAgents={async () => ({ agents })}
        loadCatalog={async () => catalog}
        createSession={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /创\s*建$/ }))
    expect((await screen.findByRole('alert')).textContent).toContain('请选择 agent')
  })

  it('submits the selected agent with the preselected catalog model', async () => {
    const createSession = vi.fn(async () => ({ key: 'oc_new' }))
    const onCreated = vi.fn()
    render(
      <SessionCreateModal
        open
        projectId="p1"
        projects={projects}
        onClose={() => {}}
        onCreated={onCreated}
        loadAgents={async () => ({ agents })}
        loadCatalog={async () => catalog}
        createSession={createSession}
      />,
    )
    await waitFor(() => expect(screen.getByRole('combobox', { name: 'Agent' })).toBeTruthy())

    fireEvent.mouseDown(screen.getByRole('combobox', { name: 'Agent' }))
    const option = await screen.findByText('Codex')
    expect(screen.getByText(/Broken（不可达/)).toBeTruthy()
    fireEvent.click(option)

    fireEvent.click(screen.getByRole('button', { name: /创\s*建$/ }))
    await waitFor(() =>
      expect(createSession).toHaveBeenCalledWith({
        agent: 'codex',
        projectId: 'p1',
        prompt: null,
        model: 'deepseek-chat',
        mode: 'ask',
      }),
    )
    expect(onCreated).toHaveBeenCalledWith('oc_new')
  })
})
