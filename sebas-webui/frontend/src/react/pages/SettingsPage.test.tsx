import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { About, AgentKindInfo, ProviderAdmin } from '../../api/client.js'
import {
  AboutPanel,
  AgentsPanel,
  EnvPanel,
  ModelsPanel,
  SELF_LOCK_TITLE,
  ServicesPanel,
  UsersPanel,
  WEAK_PASSWORD_TEXT,
  visibleSections,
} from './SettingsPage.js'

afterEach(cleanup)

const about: About = {
  uptime: '1h',
  version: '0.1.0',
  build_time: '2026-11-08 12:00',
  git_branch: 'feat/react-webui-redesign',
  git_hash: 'abc1234',
  rustc: { state: 'ok', version: 'rustc 1.90.0' },
  rustc_required: '1.85',
  router_listen: '127.0.0.1:9797',
  provider_count: 2,
  default_agent_kind: 'codex',
}

describe('settings section visibility', () => {
  it('shows Users only to root and keeps read sections for everyone', () => {
    expect(visibleSections('root').map((section) => section.id)).toContain('users')
    expect(visibleSections('admin').map((section) => section.id)).not.toContain('users')
    expect(visibleSections('viewer').map((section) => section.id)).toContain('env')
    expect(visibleSections(null).map((section) => section.id)).not.toContain('users')
  })
})

describe('AboutPanel', () => {
  it('renders real about values and an honest toolchain state', async () => {
    render(<AboutPanel load={async () => about} />)
    expect(await screen.findByText('0.1.0')).toBeTruthy()
    expect(screen.getByText('feat/react-webui-redesign · abc1234')).toBeTruthy()
    expect(screen.getByText(/rustc 1.90.0/)).toBeTruthy()
  })
})

describe('ServicesPanel', () => {
  it('degrades honestly without a watchdog adapter and hides write actions', async () => {
    render(
      <ServicesPanel
        role="admin"
        load={async () => ({
          adapter_ok: false,
          services: [],
        })}
      />,
    )
    expect(await screen.findByText(/无 watchdog 控制面/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: '停用' })).toBeNull()
  })

  it('skips the admin services request for roles without control', async () => {
    const load = vi.fn(async () => ({
      adapter_ok: true,
      services: [
        { name: 'router', status: 'running' as const, desired: 'running', uptime_secs: 5 },
      ],
    }))
    render(<ServicesPanel role="viewer" load={load} />)
    expect(await screen.findByText(/仅 root\/admin 可查看/)).toBeTruthy()
    expect(load).not.toHaveBeenCalled()
    expect(screen.queryByText(/加载失败/)).toBeNull()
  })

  it('invokes enable/disable through the injected control API', async () => {
    const enable = vi.fn(async () => ({}))
    const control = {
      enable,
      disable: vi.fn(async () => ({})),
      restart: vi.fn(async () => ({})),
    }
    render(
      <ServicesPanel
        role="admin"
        load={async () => ({
          adapter_ok: true,
          services: [{ name: 'router', status: 'stopped', desired: 'running', uptime_secs: null }],
        })}
        control={control}
      />,
    )
    fireEvent.click(await screen.findByRole('button', { name: /启\s*用/ }))
    await waitFor(() => expect(enable).toHaveBeenCalledWith('router'))
  })
})

describe('ModelsPanel', () => {
  const providers: ProviderAdmin[] = [
    {
      name: 'openrouter',
      base_url_anthropic: 'https://example.test',
      base_url_openai_chat: null,
      base_url_openai_responses: null,
      api_key_env: 'OPENROUTER_KEY',
      api_key_configured: true,
      models: [{ id: 'deepseek-chat', tags: [] }],
    },
  ]

  it('shows the catalog but no delete control for members', async () => {
    render(
      <ModelsPanel
        role="member"
        load={async () => ({ providers })}
        loadPresets={async () => ({ presets: [] })}
        loadDefaults={async () => ({ default_provider: null, default_model: null })}
      />,
    )
    expect(await screen.findByText('openrouter')).toBeTruthy()
    expect(screen.getByText('deepseek-chat')).toBeTruthy()
    expect(screen.queryByRole('button', { name: '删除' })).toBeNull()
    expect(screen.queryByRole('button', { name: '新建 provider' })).toBeNull()
  })

  it('creates a provider with the real payload shape', async () => {
    const create = vi.fn(async () => ({}))
    render(
      <ModelsPanel
        role="admin"
        load={async () => ({ providers: [] })}
        loadPresets={async () => ({ presets: [] })}
        loadDefaults={async () => ({ default_provider: null, default_model: null })}
        create={create}
      />,
    )
    fireEvent.click(await screen.findByRole('button', { name: '新建 provider' }))
    fireEvent.change(screen.getByLabelText('provider 名称'), { target: { value: 'p1' } })
    fireEvent.change(screen.getByLabelText('Anthropic base URL'), {
      target: { value: 'https://p1.test' },
    })
    fireEvent.change(screen.getByLabelText('模型列表'), { target: { value: 'm1, m2' } })
    fireEvent.click(screen.getByRole('button', { name: /创\s*建/ }))
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith({
        name: 'p1',
        base_url_anthropic: 'https://p1.test',
        models: [
          { id: 'm1', tags: [] },
          { id: 'm2', tags: [] },
        ],
      }),
    )
  })

  it('edits an existing provider and probes its model list', async () => {
    const update = vi.fn(async () => ({}))
    const probe = vi.fn(async () => ({ models: ['m9', 'm8'] }))
    render(
      <ModelsPanel
        role="admin"
        load={async () => ({ providers })}
        loadPresets={async () => ({ presets: [] })}
        loadDefaults={async () => ({ default_provider: 'openrouter', default_model: 'm1' })}
        update={update}
        fetchModels={probe}
      />,
    )
    expect(await screen.findByText(/默认：openrouter \/ m1/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /编\s*辑/ }))
    fireEvent.change(screen.getByLabelText('默认模型'), { target: { value: 'm2' } })
    fireEvent.click(screen.getByRole('button', { name: '抓取模型列表' }))
    await waitFor(() => expect(probe).toHaveBeenCalledWith('openrouter'))
    await waitFor(() =>
      expect((screen.getByLabelText('模型列表') as HTMLTextAreaElement).value).toBe('m9, m8'),
    )
    fireEvent.click(screen.getByRole('button', { name: /保\s*存/ }))
    await waitFor(() =>
      expect(update).toHaveBeenCalledWith('openrouter', {
        name: 'openrouter',
        base_url_anthropic: 'https://example.test',
        api_key_env: 'OPENROUTER_KEY',
        default_model: 'm2',
        models: [
          { id: 'm9', tags: [] },
          { id: 'm8', tags: [] },
        ],
      }),
    )
  })

  it('renders config-seeded providers as read-only', async () => {
    render(
      <ModelsPanel
        role="admin"
        load={async () => ({
          providers: [],
          config_providers: [
            {
              name: 'seed',
              base_url_anthropic: 'https://seed.test',
              base_url_openai_chat: null,
              base_url_openai_responses: null,
              source: 'config',
              editable: false,
              also_in_store: false,
            },
          ],
        })}
        loadPresets={async () => ({ presets: [] })}
        loadDefaults={async () => ({ default_provider: null, default_model: null })}
      />,
    )
    expect(await screen.findByText('seed')).toBeTruthy()
    expect(screen.getByText('config.toml · 只读')).toBeTruthy()
    expect(screen.queryByRole('button', { name: /编\s*辑/ })).toBeNull()
  })
})

describe('AgentsPanel', () => {
  const agents: AgentKindInfo[] = [
    { id: 'native', display: 'PI', reachable: true },
    {
      id: 'claude',
      display: 'claude',
      display_raw: null,
      reachable: true,
      driver_raw: 'claude',
      path_raw: 'claude',
      args: ['--flag'],
      work_dir: '/tmp/work',
      sessions_dir: null,
    },
  ]

  it('hides write controls from members and never offers native deletion', async () => {
    render(<AgentsPanel role="member" load={async () => ({ agents })} />)
    expect(await screen.findByText('内置 · PI')).toBeTruthy()
    expect(screen.queryByRole('button', { name: '新建 agent' })).toBeNull()
    expect(screen.queryByRole('button', { name: /删\s*除/ })).toBeNull()
  })

  it('creates a claude agent with the real launch payload', async () => {
    const create = vi.fn(async () => ({}))
    render(
      <AgentsPanel
        role="admin"
        load={async () => ({ agents: [agents[0]!] })}
        create={create}
      />,
    )
    fireEvent.click(await screen.findByRole('button', { name: '新建 agent' }))
    fireEvent.change(screen.getByLabelText('agent id'), { target: { value: 'myclaude' } })
    fireEvent.change(screen.getByLabelText('agent 显示名'), { target: { value: 'Mine' } })
    fireEvent.change(screen.getByLabelText('启动参数'), { target: { value: '--verbose' } })
    fireEvent.change(screen.getByLabelText('工作目录'), { target: { value: '/tmp/w' } })
    fireEvent.click(screen.getByRole('button', { name: /创\s*建/ }))
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith('myclaude', {
        driver: 'claude',
        path: 'claude',
        args: ['--verbose'],
        display: 'Mine',
        work_dir: '/tmp/w',
        sessions_dir: null,
      }),
    )
  })

  it('edits an existing agent and preserves its launch shape', async () => {
    const update = vi.fn(async () => ({}))
    render(
      <AgentsPanel role="admin" load={async () => ({ agents })} update={update} />,
    )
    await screen.findByText('claude')
    fireEvent.click(screen.getByRole('button', { name: /编\s*辑/ }))
    expect((screen.getByLabelText('agent id') as HTMLInputElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('agent 显示名'), { target: { value: 'Renamed' } })
    fireEvent.click(screen.getByRole('button', { name: /保\s*存/ }))
    await waitFor(() =>
      expect(update).toHaveBeenCalledWith('claude', {
        display: 'Renamed',
        args: ['--flag'],
        work_dir: '/tmp/work',
        sessions_dir: null,
      }),
    )
  })
})

describe('UsersPanel', () => {
  it('guards self role/delete controls while keeping password reset', async () => {
    render(
      <UsersPanel
        username="root"
        load={async () => ({
          users: [
            { id: 1, username: 'root', role: 'root', enabled: true, created_at_unix: 0 },
            { id: 2, username: 'ops', role: 'admin', enabled: true, created_at_unix: 0 },
          ],
        })}
      />,
    )
    await screen.findByText('ops')
    const remove = screen.queryAllByRole('button', { name: /删\s*除/ })[0]!
    expect(remove.hasAttribute('disabled')).toBe(true)
    // A disabled control must name its reason (self-lock guard).
    expect(remove.getAttribute('title')).toBe(SELF_LOCK_TITLE)
    const stop = screen.getAllByRole('button', { name: /停\s*用/ })[0]!
    expect(stop.hasAttribute('disabled')).toBe(true)
    expect(stop.getAttribute('title')).toBe(SELF_LOCK_TITLE)
    // The self row's role selector is locked for the same reason. AntD puts a
    // `title` passed to Select on the control wrapper (the advertised combobox
    // is the inner input), so the reason is asserted on that wrapper.
    const selfRole = screen.getByRole('combobox', { name: 'root 的角色' })
    expect(selfRole.hasAttribute('disabled')).toBe(true)
    expect(selfRole.closest('.ant-select')?.getAttribute('title')).toBe(SELF_LOCK_TITLE)
    // Other rows stay actionable, and password reset is never self-locked.
    const otherRole = screen.getByRole('combobox', { name: 'ops 的角色' })
    expect(otherRole.hasAttribute('disabled')).toBe(false)
    expect(screen.getAllByRole('button', { name: '重置密码' })).toHaveLength(2)
  })

  it('blocks a weak password client-side without calling the API', async () => {
    const create = vi.fn(async () => ({}))
    render(
      <UsersPanel
        username="root"
        load={async () => ({
          users: [{ id: 1, username: 'root', role: 'root', enabled: true, created_at_unix: 0 }],
        })}
        mutate={{
          create,
          role: async () => ({}),
          enabled: async () => ({}),
          password: async () => ({}),
          remove: async () => ({}),
        }}
      />,
    )
    fireEvent.click(await screen.findByRole('button', { name: '新建用户' }))
    fireEvent.change(screen.getByLabelText('新用户名'), { target: { value: 'shortlived' } })
    fireEvent.change(screen.getByLabelText('新用户密码'), { target: { value: 'short' } })
    fireEvent.click(screen.getByRole('button', { name: /创\s*建/ }))
    expect(create).not.toHaveBeenCalled()
    // The message appears both in the (still open) dialog and in the panel's
    // action slot, so assert on the dialog's own alert.
    const dialog = screen.getByRole('dialog')
    expect(within(dialog).getByText(WEAK_PASSWORD_TEXT)).toBeTruthy()
    // The dialog stays open so the operator can fix the value.
    expect(dialog).toBeTruthy()
  })

  it('deletes through a named confirmation dialog instead of one click', async () => {
    const remove = vi.fn(async () => ({}))
    render(
      <UsersPanel
        username="root"
        load={async () => ({
          users: [
            { id: 1, username: 'root', role: 'root', enabled: true, created_at_unix: 0 },
            { id: 2, username: 'tester', role: 'member', enabled: true, created_at_unix: 0 },
          ],
        })}
        mutate={{
          create: async () => ({}),
          role: async () => ({}),
          enabled: async () => ({}),
          password: async () => ({}),
          remove,
        }}
      />,
    )
    await screen.findByText('tester')
    fireEvent.click(screen.getAllByRole('button', { name: /删\s*除/ })[1]!)
    // AntD's test-env `useId` stub returns one constant id for every mounted
    // Modal, so name-by-`aria-labelledby` collides across the panel's dialogs.
    // The real app has unique ids; here we assert the single open dialog and
    // its copy directly.
    const dialog = await screen.findByRole('dialog')
    expect(dialog.textContent).toContain('删除用户')
    expect(dialog.textContent).toContain('tester')
    expect(remove).not.toHaveBeenCalled()
    fireEvent.click(within(dialog).getByRole('button', { name: /删\s*除/ }))
    await waitFor(() => expect(remove).toHaveBeenCalledWith(2))
  })
})

describe('EnvPanel', () => {
  it('masks sensitive values and labels undefined state honestly', async () => {
    render(
      <EnvPanel
        load={async () => ({
          items: [
            {
              name: 'SEBAS_HOME',
              what: 'home',
              kind: 'plain',
              value: '/tmp/home',
            },
            {
              name: 'SECRET',
              what: 'token',
              kind: 'set_unset',
              value: null,
              set: true,
            },
            {
              name: 'UNKNOWN',
              what: 'token',
              kind: 'set_unset',
              value: null,
            },
          ],
        })}
      />,
    )
    expect(await screen.findByText('/tmp/home')).toBeTruthy()
    expect(screen.getByText('已设置（已遮蔽）')).toBeTruthy()
    expect(screen.getByText('无法确定')).toBeTruthy()
    expect(screen.queryByText('hunter2')).toBeNull()
  })
})
