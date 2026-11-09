import { useEffect, useMemo, useState, type ReactNode } from 'react'
import {
  Alert,
  Button,
  Card,
  Empty,
  Input,
  Modal,
  Select,
  Space,
  Spin,
  Tag,
  Typography,
} from 'antd'
import {
  api,
  errorText,
  MODEL_SLOTS,
  ROLES,
  type About,
  type AdminService,
  type AgentKindInfo,
  type AgentLaunchPayload,
  type ConfigSeededProvider,
  type EnvVarEntry,
  type ModelAliasEntry,
  type ModelSlot,
  type ModelSlotMapping,
  type ProviderAdmin,
  type ProviderModelEntry,
  type ProviderPayload,
  type ProviderPreset,
  type Role,
  type SkillDetail,
  type SkillEntry,
  type SkillsSyncResponse,
  type UserRecord,
} from '../../api/client.js'
import {
  canControlServices,
  canManageAgents,
  canManageProviders,
  canManageSkills,
  canManageUsers,
} from '../../views/role-visibility.js'
import { renderMarkdown } from '../../components/markdown.js'
import { stripFrontmatter } from '../../views/skill-frontmatter.js'
import {
  readLastSettingsSection,
  writeLastSettingsSection,
  type SettingsSectionId as StoredSectionId,
} from '../boundary/preferences.js'
import { useThemeMode } from '../theme/ThemeProvider.js'
import styles from './settings.module.css'

export type SettingsSectionId =
  | 'general'
  | 'appearance'
  | 'services'
  | 'users'
  | 'models'
  | 'aliases'
  | 'agents'
  | 'skills'
  | 'env'
  | 'about'

interface SectionDef {
  id: SettingsSectionId
  label: string
  visible: (role: Role | null) => boolean
}

export const SETTINGS_SECTIONS: readonly SectionDef[] = [
  { id: 'general', label: '通用', visible: () => true },
  { id: 'appearance', label: '外观', visible: () => true },
  { id: 'services', label: '服务', visible: () => true },
  { id: 'users', label: '用户', visible: (role) => canManageUsers(role) },
  { id: 'models', label: '模型', visible: () => true },
  { id: 'aliases', label: '别名', visible: () => true },
  { id: 'agents', label: 'Agent', visible: () => true },
  { id: 'skills', label: '技能', visible: () => true },
  { id: 'env', label: '环境变量', visible: () => true },
  { id: 'about', label: '关于', visible: () => true },
]

export function visibleSections(role: Role | null): SectionDef[] {
  return SETTINGS_SECTIONS.filter((section) => section.visible(role))
}

/** Persisted section vocabulary ↔ this panel's ids. */
const REMEMBERED_TO_SECTION: Record<string, SettingsSectionId> = {
  generic: 'general',
  appearance: 'appearance',
  services: 'services',
  users: 'users',
  models: 'models',
  aliases: 'aliases',
  agents: 'agents',
  skills: 'skills',
  'env-vars': 'env',
  about: 'about',
}

/**
 * Section remembered from the previous visit. Unknown/legacy values (e.g. the
 * pre-rename `settings`) fall back to General instead of an empty panel.
 */
function rememberedSection(): SettingsSectionId | null {
  return REMEMBERED_TO_SECTION[readLastSettingsSection()] ?? null
}

function writeRememberedSection(section: SettingsSectionId): void {
  const entry = Object.entries(REMEMBERED_TO_SECTION).find(([, id]) => id === section)
  writeLastSettingsSection((entry?.[0] ?? 'generic') as StoredSectionId)
}

export interface SettingsPageProps {
  role: Role | null
  username: string | null
  onClose: () => void
}

/** Settings workspace: sections without a dedicated URL (3.6). */
export function SettingsPage({ role, username, onClose }: SettingsPageProps) {
  const sections = visibleSections(role)
  const remembered = rememberedSection()
  const [active, setActive] = useState<SettingsSectionId>(
    remembered ?? sections[0]?.id ?? 'general',
  )
  const [visited, setVisited] = useState<ReadonlySet<SettingsSectionId>>(
    () => new Set([remembered ?? sections[0]?.id ?? 'general']),
  )

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  useEffect(() => {
    if (!sections.some((section) => section.id === active)) {
      setActive(sections[0]?.id ?? 'general')
    }
  }, [sections, active])

  function open(section: SettingsSectionId): void {
    setActive(section)
    setVisited((prev) => new Set(prev).add(section))
    writeRememberedSection(section)
  }

  return (
    <section className={styles.workspace} aria-label="设置">
      <aside className={styles.nav} aria-label="设置分区">
        <div className={styles.navHead}>
          <Typography.Title level={4} className={styles.navTitle}>
            设置
          </Typography.Title>
          <Button size="small" onClick={onClose} aria-label="关闭设置">
            关闭
          </Button>
        </div>
        <nav>
          {sections.map((section) => (
            <button
              key={section.id}
              type="button"
              className={styles.navItem}
              aria-current={active === section.id ? 'page' : undefined}
              onClick={() => open(section.id)}
            >
              {section.label}
            </button>
          ))}
        </nav>
        <Typography.Text type="secondary" className={styles.navFoot}>
          {username ?? '未登录'} · {role ?? '免登录'}
        </Typography.Text>
      </aside>

      <div className={styles.content}>
        {visited.has('general') ? <Panel hidden={active !== 'general'}><GeneralPanel /></Panel> : null}
        {visited.has('appearance') ? <Panel hidden={active !== 'appearance'}><AppearancePanel /></Panel> : null}
        {visited.has('services') ? <Panel hidden={active !== 'services'}><ServicesPanel role={role} /></Panel> : null}
        {visited.has('users') ? <Panel hidden={active !== 'users'}><UsersPanel username={username} /></Panel> : null}
        {visited.has('models') ? <Panel hidden={active !== 'models'}><ModelsPanel role={role} /></Panel> : null}
        {visited.has('aliases') ? <Panel hidden={active !== 'aliases'}><AliasesPanel role={role} /></Panel> : null}
        {visited.has('agents') ? <Panel hidden={active !== 'agents'}><AgentsPanel role={role} /></Panel> : null}
        {visited.has('skills') ? <Panel hidden={active !== 'skills'}><SkillsPanel role={role} /></Panel> : null}
        {visited.has('env') ? <Panel hidden={active !== 'env'}><EnvPanel /></Panel> : null}
        {visited.has('about') ? <Panel hidden={active !== 'about'}><AboutPanel /></Panel> : null}
      </div>
    </section>
  )
}

function Panel({ hidden, children }: { hidden: boolean; children: ReactNode }) {
  return (
    <div className={styles.panel} hidden={hidden}>
      {children}
    </div>
  )
}

function SectionHead({ title, hint }: { title: string; hint?: string }) {
  return (
    <header className={styles.head}>
      <Typography.Title level={4}>{title}</Typography.Title>
      {hint ? <Typography.Paragraph type="secondary">{hint}</Typography.Paragraph> : null}
    </header>
  )
}

function LoadError({ message }: { message: string }) {
  return <Alert type="error" showIcon title={`加载失败：${message}`} role="alert" />
}

// ─── General / About ────────────────────────────────────────────────────────

/** General holds preferences; there is nothing configurable on this surface yet. */
export function GeneralPanel() {
  return (
    <>
      <SectionHead title="通用" hint="通用偏好将在后续版本提供。" />
      <Typography.Paragraph type="secondary">
        暂无通用偏好。语言切换等其它偏好以后会在这里提供。
      </Typography.Paragraph>
    </>
  )
}

/** About = instance facts and build truth from `/api/about` (blueprint §4.4). */
export function AboutPanel({ load = api.about }: { load?: () => Promise<About> }) {
  const [about, setAbout] = useState<About | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let cancelled = false
    load()
      .then((value) => {
        if (!cancelled) setAbout(value)
      })
      .catch((failure: unknown) => {
        if (!cancelled) setError(errorText(failure))
      })
    return () => {
      cancelled = true
    }
  }, [load])
  return (
    <>
      <SectionHead title="关于" hint="实例与构建信息，全部来自 /api/about 真值。" />
      {error !== null ? <LoadError message={error} /> : null}
      {about === null && error === null ? <Spin /> : null}
      {about !== null ? (
        <Card variant="borderless">
          <dl className={styles.facts}>
            <dt>版本</dt><dd>{about.version}</dd>
            <dt>运行时长</dt><dd>{about.uptime}</dd>
            <dt>构建时间</dt><dd>{about.build_time}</dd>
            <dt>Git</dt><dd>{about.git_branch} · {about.git_hash}</dd>
            <dt>默认 agent</dt><dd>{about.default_agent_kind}</dd>
            <dt>router</dt><dd>{about.router_listen ?? '未监听'}</dd>
            <dt>provider 数</dt><dd>{about.provider_count}</dd>
            <dt>Rust</dt>
            <dd>
              {about.rustc.state === 'ok'
                ? about.rustc.version ?? 'ok'
                : `${about.rustc.state}${about.rustc.cause ? `：${about.rustc.cause}` : ''}`}
              （要求 {about.rustc_required}）
            </dd>
          </dl>
        </Card>
      ) : null}
    </>
  )
}

// ─── Appearance ─────────────────────────────────────────────────────────────

function AppearancePanel() {
  const { mode, resolved, setMode } = useThemeMode()
  return (
    <>
      <SectionHead title="外观" hint="立即生效，只保存在当前浏览器。" />
      <Space wrap>
        {[
          { value: 'system', label: '跟随系统' },
          { value: 'light', label: '浅色' },
          { value: 'dark', label: '深色' },
        ].map((option) => (
          <Button
            key={option.value}
            type={mode === option.value ? 'primary' : 'default'}
            aria-pressed={mode === option.value}
            onClick={() => setMode(option.value as 'system' | 'light' | 'dark')}
          >
            {option.label}
          </Button>
        ))}
      </Space>
      <Typography.Paragraph type="secondary" className={styles.hint}>
        当前解析为{resolved === 'light' ? '浅色' : '深色'}。
      </Typography.Paragraph>
    </>
  )
}

// ─── Services ───────────────────────────────────────────────────────────────

/** The watchdog only supervises these four rows; leaked watchdog/updater
 * synthetic entries are ignored at render time as a second fence. */
const MANAGED_SERVICE_ROWS = new Set(['core', 'webui', 'router', 'im'])

type ServiceVerb = 'enable' | 'disable' | 'restart'

/** Status-driven action matrix (status-driven-service-rows): a row exposes
 * only the transitions that make sense for its actual state, and transitional
 * or unknown states stay non-interactive. */
function serviceActions(status: string): ServiceVerb[] {
  switch (status) {
    case 'running':
      return ['disable']
    case 'stopped':
    case 'disabled':
      return ['enable']
    case 'degraded':
    case 'failed-startup':
      return ['disable', 'restart']
    default:
      return []
  }
}

export function ServicesPanel({
  role,
  load = api.adminServicesSafe,
  control = {
    enable: api.enableService,
    disable: api.disableService,
    restart: api.restartService,
  },
}: {
  role: Role | null
  load?: () => Promise<{ adapter_ok: boolean; services: AdminService[] }>
  control?: {
    enable: (name: string) => Promise<unknown>
    disable: (name: string, force?: boolean) => Promise<unknown>
    restart: (name: string) => Promise<unknown>
  }
}) {
  const [data, setData] = useState<{ adapter_ok: boolean; services: AdminService[] } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [action, setAction] = useState<{ ok: boolean; text: string } | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [forceStop, setForceStop] = useState<{ name: string; count: number | null } | null>(null)
  const writable = canControlServices(role)

  async function reload(): Promise<void> {
    try {
      setData(await load())
      setError(null)
    } catch (failure) {
      setError(errorText(failure))
    }
  }
  useEffect(() => {
    // `/api/admin/services` is root/admin-only. Loading it for a viewer (or any
    // other role without control) only yields a 403 and hides the read-only
    // explanation behind the error state, so skip the request entirely.
    if (!writable) {
      setData(null)
      setError(null)
      setAction(null)
      setBusy(null)
      return
    }
    void reload()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load, writable])

  async function run(name: string, verb: ServiceVerb, force = false): Promise<void> {
    setBusy(name)
    setAction(null)
    try {
      await (verb === 'enable'
        ? control.enable(name)
        : verb === 'disable'
          ? control.disable(name, force)
          : control.restart(name))
      setForceStop(null)
      setAction({ ok: true, text: `${name}：${verb} 已受理` })
      await reload()
    } catch (failure) {
      const code = (failure as { code?: unknown }).code
      const rawCount = (failure as { count?: unknown }).count
      const count = typeof rawCount === 'number' ? rawCount : null
      if (verb === 'disable' && code === 'active_routed_sessions' && !force) {
        setForceStop({ name, count })
      } else {
        setAction({ ok: false, text: `${name}：${errorText(failure)}` })
      }
    } finally {
      setBusy(null)
    }
  }

  return (
    <>
      <SectionHead title="服务" hint="受管子进程；无控制面时如实降级。" />
      {error !== null ? <LoadError message={error} /> : null}
      {action !== null ? (
        <Alert type={action.ok ? 'success' : 'error'} showIcon title={action.text} />
      ) : null}
      {!writable ? (
        <Alert
          type="info"
          showIcon
          title="服务列表与启停仅 root/admin 可查看；服务端 403 仍是防线。"
        />
      ) : null}
      {writable && data === null && error === null ? <Spin /> : null}
      {writable && data !== null && !data.adapter_ok ? (
        <Alert
          type="info"
          showIcon
          title="无 watchdog 控制面：受管服务列表不可用。请以 `sebas run` 启动后再打开。"
        />
      ) : null}
      {writable ? data?.services
        .filter((service) => MANAGED_SERVICE_ROWS.has(service.name))
        .map((service) => {
          const readOnly = service.name === 'core'
          const verbs = readOnly ? [] : serviceActions(service.status)
          return (
            <div key={service.name} className={styles.row}>
              <div>
                <strong>{service.name}</strong>
                <span className={styles.meta}>
                  desired={service.desired} · actual={service.status}
                  {service.uptime_secs === null ? '' : ` · ${service.uptime_secs}s`}
                </span>
              </div>
              {readOnly ? (
                <Tag>只读</Tag>
              ) : verbs.length > 0 ? (
                <Space>
                  {verbs.map((verb) => (
                    <Button
                      key={verb}
                      size="small"
                      danger={verb === 'disable'}
                      loading={busy === service.name}
                      onClick={() => void run(service.name, verb)}
                    >
                      {verb === 'enable' ? '启用' : verb === 'disable' ? '停用' : '重启'}
                    </Button>
                  ))}
                </Space>
              ) : null}
            </div>
          )
        }) : null}
      <Modal
        open={forceStop !== null}
        title="强制停止服务"
        okText="强制停止"
        cancelText="取消"
        confirmLoading={forceStop !== null && busy === forceStop.name}
        onCancel={() => setForceStop(null)}
        onOk={() => {
          if (forceStop !== null) void run(forceStop.name, 'disable', true)
        }}
      >
        {forceStop !== null ? (
          <p>
            {forceStop.name} 仍有 {String(forceStop.count ?? '若干')} 个 routed
            会话；强制停止会中断这些会话。
          </p>
        ) : null}
      </Modal>
    </>
  )
}

// ─── Users ──────────────────────────────────────────────────────────────────

/** Minimum password length enforced client-side before any POST (server 400 too). */
export const MIN_PASSWORD_LENGTH = 8
export const WEAK_PASSWORD_TEXT = '密码至少需要 8 个字符'
/** Self-lock guard copy: a row acting on itself is disabled AND explains why. */
export const SELF_LOCK_TITLE = '不能对自己行执行该操作（自锁防护）'

export function UsersPanel({
  username,
  load = api.usersList,
  mutate = {
    create: api.usersCreate,
    role: api.usersSetRole,
    enabled: api.usersSetEnabled,
    password: api.usersSetPassword,
    remove: api.usersDelete,
  },
}: {
  username: string | null
  load?: () => Promise<{ users: UserRecord[] }>
  mutate?: {
    create: (username: string, password: string, role: Role) => Promise<unknown>
    role: (id: number, role: Role) => Promise<unknown>
    enabled: (id: number, enabled: boolean) => Promise<unknown>
    password: (id: number, password: string) => Promise<unknown>
    remove: (id: number) => Promise<unknown>
  }
}) {
  const [users, setUsers] = useState<UserRecord[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [action, setAction] = useState<{ ok: boolean; text: string } | null>(null)
  const [createOpen, setCreateOpen] = useState(false)
  const [form, setForm] = useState({ username: '', password: '', role: 'member' as Role })
  const [createError, setCreateError] = useState<string | null>(null)
  const [reset, setReset] = useState<UserRecord | null>(null)
  const [password, setPassword] = useState('')
  const [remove, setRemove] = useState<UserRecord | null>(null)
  const [busy, setBusy] = useState(false)

  async function reload(): Promise<void> {
    try {
      const response = await load()
      setUsers(response.users)
      setError(null)
    } catch (failure) {
      setError(errorText(failure))
    }
  }
  useEffect(() => {
    void reload()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load])

  async function act(text: string, run: () => Promise<unknown>): Promise<void> {
    setBusy(true)
    setAction(null)
    try {
      await run()
      setAction({ ok: true, text })
      await reload()
    } catch (failure) {
      setAction({ ok: false, text: errorText(failure) })
    } finally {
      setBusy(false)
    }
  }

  function openCreate(): void {
    setCreateError(null)
    setCreateOpen(true)
  }

  /**
   * Client-side gate: a too-short password never reaches the API. The server
   * still validates (defense in depth), but firing a request the operator can
   * only fix from a 400 wastes a round trip — and the dialog stays open.
   */
  function submitCreate(): void {
    const name = form.username.trim()
    if (form.password.length < MIN_PASSWORD_LENGTH) {
      // One message, in place: the dialog stays open and no request leaves.
      // The settings-level action slot carries it too (the dialog is portaled
      // away from the region, so the operator sees it wherever they look).
      setCreateError(WEAK_PASSWORD_TEXT)
      setAction({ ok: false, text: WEAK_PASSWORD_TEXT })
      return
    }
    setCreateError(null)
    void act('用户已创建', async () => {
      await mutate.create(name, form.password, form.role)
      setCreateOpen(false)
      setForm({ username: '', password: '', role: 'member' })
    })
  }

  return (
    <>
      <SectionHead title="用户" hint="仅 root 可见；服务端 403 仍是最终防线。" />
      <Space>
        <Button onClick={openCreate}>新建用户</Button>
      </Space>
      {error !== null ? <LoadError message={error} /> : null}
      {action !== null ? (
        <Alert type={action.ok ? 'success' : 'error'} showIcon title={action.text} />
      ) : null}
      {users === null && error === null ? <Spin /> : null}
      {users?.length === 0 ? <Empty description="还没有用户" /> : null}
      {users?.map((user) => {
        const self = username !== null && user.username === username
        return (
          <div key={user.id} className={styles.row}>
            <div>
              <strong>{user.username}</strong>{' '}
              <Tag>{user.role}</Tag>
              <span className={styles.meta}>
                {user.enabled ? '已启用' : '已禁用'} · 创建于{' '}
                {new Date(user.created_at_unix * 1000).toLocaleDateString('zh-CN')}
              </span>
            </div>
            <Space wrap>
              <Select
                size="small"
                aria-label={`${user.username} 的角色`}
                value={user.role}
                disabled={busy || self}
                title={self ? SELF_LOCK_TITLE : undefined}
                options={ROLES.map((role) => ({ value: role, label: role }))}
                onChange={(role) => void act(`${user.username} 角色更新`, () => mutate.role(user.id, role))}
              />
              <Button
                size="small"
                disabled={busy || self}
                title={self ? SELF_LOCK_TITLE : undefined}
                onClick={() =>
                  void act(user.enabled ? '已停用用户' : '已启用用户', () =>
                    mutate.enabled(user.id, !user.enabled),
                  )
                }
              >
                {user.enabled ? '停用' : '启用'}
              </Button>
              <Button size="small" disabled={busy} onClick={() => setReset(user)}>
                重置密码
              </Button>
              <Button
                size="small"
                danger
                disabled={busy || self}
                title={self ? SELF_LOCK_TITLE : undefined}
                onClick={() => setRemove(user)}
              >
                删除
              </Button>
            </Space>
          </div>
        )
      })}

      <Modal
        open={createOpen}
        title="新建用户"
        okText="创建"
        cancelText="取消"
        confirmLoading={busy}
        onCancel={() => setCreateOpen(false)}
        onOk={submitCreate}
      >
        <Space orientation="vertical" className={styles.form}>
          <Input
            aria-label="新用户名"
            placeholder="用户名"
            value={form.username}
            onChange={(event) => setForm({ ...form, username: event.target.value })}
          />
          <Input.Password
            aria-label="新用户密码"
            placeholder="密码（至少 8 个字符）"
            value={form.password}
            onChange={(event) => {
              setForm({ ...form, password: event.target.value })
              setCreateError(null)
            }}
          />
          {createError !== null ? <Alert type="error" showIcon title={createError} /> : null}
          <Select
            aria-label="新用户角色"
            value={form.role}
            options={ROLES.map((role) => ({ value: role, label: role }))}
            onChange={(role) => setForm({ ...form, role })}
          />
        </Space>
      </Modal>

      <Modal
        open={remove !== null}
        title="删除用户"
        okText="删除"
        okButtonProps={{ danger: true }}
        cancelText="取消"
        confirmLoading={busy}
        onCancel={() => setRemove(null)}
        onOk={() =>
          void act('已删除用户', async () => {
            if (remove !== null) await mutate.remove(remove.id)
            setRemove(null)
          })
        }
      >
        <Typography.Paragraph>
          删除用户「{remove?.username ?? ''}」后其凭据立即失效，且不可恢复。确定删除？
        </Typography.Paragraph>
      </Modal>

      <Modal
        open={reset !== null}
        title={`重置密码：${reset?.username ?? ''}`}
        okText="保存"
        cancelText="取消"
        confirmLoading={busy}
        onCancel={() => setReset(null)}
        onOk={() =>
          void act('密码已重置', async () => {
            if (reset !== null) await mutate.password(reset.id, password)
            setReset(null)
            setPassword('')
          })
        }
      >
        <Input.Password
          aria-label="新密码"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />
      </Modal>
    </>
  )
}

// ─── Models ─────────────────────────────────────────────────────────────────

interface ProviderForm {
  mode: 'create' | 'edit'
  name: string
  preset: string | null
  anthropic: string
  openai: string
  responses: string
  keyEnv: string
  defaultModel: string
  models: string
}

function emptyProviderForm(): ProviderForm {
  return {
    mode: 'create',
    name: '',
    preset: null,
    anthropic: '',
    openai: '',
    responses: '',
    keyEnv: '',
    defaultModel: '',
    models: '',
  }
}

function modelText(models: readonly ProviderModelEntry[]): string {
  return models.map((model) => model.id).join(', ')
}

function modelEntries(text: string, previous: readonly ProviderModelEntry[]): ProviderModelEntry[] {
  const tags = new Map(previous.map((model) => [model.id, model.tags]))
  return text
    .split(',')
    .map((id) => id.trim())
    .filter(Boolean)
    .map((id) => ({ id, tags: tags.get(id) ?? [] }))
}

/** Provider list, editor and config-seeded read-only rows (3.6 complete). */
export function ModelsPanel({
  role,
  load = api.providers,
  loadPresets = api.providerPresets,
  loadDefaults = api.providerDefaults,
  create = api.providerCreate,
  update = api.providerUpdate,
  remove = api.providerDelete,
  fetchModels = api.fetchProviderModels,
}: {
  role: Role | null
  load?: () => Promise<{
    providers: ProviderAdmin[]
    config_providers?: ConfigSeededProvider[]
  }>
  loadPresets?: () => Promise<{ presets: ProviderPreset[] }>
  loadDefaults?: () => Promise<{ default_provider: string | null; default_model: string | null }>
  create?: (payload: ProviderPayload) => Promise<unknown>
  update?: (name: string, payload: ProviderPayload) => Promise<unknown>
  remove?: (name: string) => Promise<unknown>
  fetchModels?: (name: string) => Promise<{ models: string[] }>
}) {
  const [providers, setProviders] = useState<ProviderAdmin[] | null>(null)
  const [configProviders, setConfigProviders] = useState<ConfigSeededProvider[]>([])
  const [presets, setPresets] = useState<ProviderPreset[]>([])
  const [defaults, setDefaults] = useState<{ provider: string | null; model: string | null }>({
    provider: null,
    model: null,
  })
  const [error, setError] = useState<string | null>(null)
  const [action, setAction] = useState<{ ok: boolean; text: string } | null>(null)
  const [form, setForm] = useState<ProviderForm | null>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const writable = canManageProviders(role)

  async function reload(): Promise<void> {
    try {
      const response = await load()
      setProviders(response.providers)
      setConfigProviders(response.config_providers ?? [])
      setError(null)
    } catch (failure) {
      setProviders([])
      setError(errorText(failure))
    }
  }

  useEffect(() => {
    void reload()
    void loadPresets()
      .then((response) => setPresets(response.presets))
      .catch(() => setPresets([]))
    void loadDefaults()
      .then((response) =>
        setDefaults({ provider: response.default_provider, model: response.default_model }),
      )
      .catch(() => setDefaults({ provider: null, model: null }))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load, loadPresets, loadDefaults])

  function openCreate(): void {
    setFormError(null)
    setForm(emptyProviderForm())
  }

  function openEdit(provider: ProviderAdmin): void {
    setFormError(null)
    setForm({
      mode: 'edit',
      name: provider.name,
      preset: provider.preset ?? null,
      anthropic: provider.base_url_anthropic ?? '',
      openai: provider.base_url_openai_chat ?? '',
      responses: provider.base_url_openai_responses ?? '',
      keyEnv: provider.api_key_env ?? '',
      defaultModel: provider.default_model ?? '',
      models: modelText(provider.models),
    })
  }

  function applyPreset(name: string | null): void {
    setForm((current) => {
      if (current === null) return current
      if (name === null) return { ...current, preset: null }
      const preset = presets.find((item) => item.name === name)
      if (preset === undefined) return { ...current, preset: name }
      return {
        ...current,
        preset: name,
        name: current.mode === 'create' ? preset.name : current.name,
        anthropic: preset.base_url_anthropic ?? '',
        openai: preset.base_url_openai_chat ?? '',
        responses: preset.base_url_openai_responses ?? '',
        keyEnv: preset.api_key_env,
        models: modelText(preset.models),
        defaultModel: preset.models[0]?.id ?? '',
      }
    })
  }

  async function save(): Promise<void> {
    if (form === null || busy) return
    const name = form.name.trim()
    if (name === '') {
      setFormError('provider 名称必填')
      return
    }
    if (!form.anthropic.trim() && !form.openai.trim() && !form.responses.trim()) {
      setFormError('自定义 provider 至少需要填写一个 Base URL')
      return
    }
    const previous = providers?.find((provider) => provider.name === name)?.models ?? []
    const payload: ProviderPayload = {
      name,
      ...(form.preset ? { preset: form.preset } : {}),
      ...(form.anthropic.trim() ? { base_url_anthropic: form.anthropic.trim() } : {}),
      ...(form.openai.trim() ? { base_url_openai_chat: form.openai.trim() } : {}),
      ...(form.responses.trim() ? { base_url_openai_responses: form.responses.trim() } : {}),
      ...(form.keyEnv.trim() ? { api_key_env: form.keyEnv.trim() } : {}),
      ...(form.defaultModel.trim() ? { default_model: form.defaultModel.trim() } : {}),
      ...(form.models.trim() ? { models: modelEntries(form.models, previous) } : {}),
    }
    setBusy(true)
    setFormError(null)
    try {
      if (form.mode === 'create') {
        await create(payload)
        setAction({ ok: true, text: `已创建 ${name}` })
      } else {
        await update(name, payload)
        setAction({ ok: true, text: `已更新 ${name}` })
      }
      setForm(null)
      await reload()
    } catch (failure) {
      setFormError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function act(text: string, run: () => Promise<unknown>): Promise<void> {
    setBusy(true)
    setAction(null)
    try {
      await run()
      setAction({ ok: true, text })
      await reload()
    } catch (failure) {
      setAction({ ok: false, text: errorText(failure) })
    } finally {
      setBusy(false)
    }
  }

  async function probeModels(): Promise<void> {
    if (form === null || form.name.trim() === '') return
    setBusy(true)
    setFormError(null)
    try {
      const response = await fetchModels(form.name.trim())
      setForm((current) => (current ? { ...current, models: response.models.join(', ') } : current))
    } catch (failure) {
      setFormError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <SectionHead title="模型" hint="Provider 目录；凭据只显示是否已配置，config.toml 行为只读。" />
      <Space wrap>
        {writable ? <Button onClick={openCreate}>新建 provider</Button> : null}
        <span className={styles.meta}>
          {defaults.provider
            ? `默认：${defaults.provider}${defaults.model ? ` / ${defaults.model}` : ''}`
            : '未设置默认'}
        </span>
      </Space>
      {error !== null ? <LoadError message={error} /> : null}
      {action !== null ? (
        <Alert type={action.ok ? 'success' : 'error'} showIcon title={action.text} />
      ) : null}
      {providers === null && error === null ? <Spin /> : null}
      {providers?.length === 0 && configProviders.length === 0 ? (
        <Empty description="还没有 provider" />
      ) : null}
      {providers?.map((provider) => {
        const url =
          provider.base_url_anthropic ??
          provider.base_url_openai_chat ??
          provider.base_url_openai_responses
        return (
          <div key={provider.name} className={styles.row}>
            <div>
              <strong>{provider.name}</strong>{' '}
              <Tag>{provider.preset ? `${provider.preset} · 内置` : '自定义'}</Tag>
              <Tag color={provider.api_key_configured ? 'green' : 'default'}>
                {provider.api_key_configured ? '已配 key' : '未配 key'}
              </Tag>
              {defaults.provider === provider.name ? <Tag color="blue">默认</Tag> : null}
              <span className={styles.meta}>
                {url ?? '无 base URL'}
                {provider.default_model ? ` · 默认模型 ${provider.default_model}` : ''}
              </span>
              <div className={styles.chips}>
                {provider.models.map((model) => (
                  <Tag key={model.id}>
                    {model.id}
                    {model.tags.length ? ` · ${model.tags.join(' ')}` : ''}
                  </Tag>
                ))}
              </div>
            </div>
            {writable ? (
              <Space size={4}>
                <Button size="small" onClick={() => openEdit(provider)}>
                  编辑
                </Button>
                <Button
                  size="small"
                  danger
                  loading={busy}
                  onClick={() => void act(`已删除 ${provider.name}`, () => remove(provider.name))}
                >
                  删除
                </Button>
              </Space>
            ) : null}
          </div>
        )
      })}
      {configProviders.map((provider) => (
        <div key={`config-${provider.name}`} className={styles.row}>
          <div>
            <strong>{provider.name}</strong> <Tag>config.toml · 只读</Tag>
            {provider.also_in_store ? <Tag color="blue">store 同名行生效中</Tag> : null}
            <span className={styles.meta}>
              {provider.base_url_anthropic ??
                provider.base_url_openai_chat ??
                provider.base_url_openai_responses ??
                '无 base URL'}
            </span>
          </div>
        </div>
      ))}

      <Modal
        open={form !== null}
        title={form?.mode === 'edit' ? `编辑 provider：${form.name}` : '新建 provider'}
        okText={form?.mode === 'edit' ? '保存' : '创建'}
        cancelText="取消"
        confirmLoading={busy}
        onCancel={() => setForm(null)}
        onOk={() => void save()}
      >
        {form !== null ? (
          <Space orientation="vertical" className={styles.form}>
            {form.mode === 'create' ? (
              <Select
                aria-label="内置 preset"
                allowClear
                value={form.preset ?? undefined}
                placeholder="选择内置 preset（可选）"
                options={presets.map((preset) => ({ value: preset.name, label: preset.name }))}
                onChange={(value) => applyPreset(value ?? null)}
              />
            ) : null}
            <Input
              aria-label="provider 名称"
              placeholder="名称"
              disabled={form.mode === 'edit'}
              value={form.name}
              onChange={(event) => setForm({ ...form, name: event.target.value })}
            />
            <Input
              aria-label="Anthropic base URL"
              placeholder="Anthropic base URL（可选）"
              value={form.anthropic}
              onChange={(event) => setForm({ ...form, anthropic: event.target.value })}
            />
            <Input
              aria-label="OpenAI base URL"
              placeholder="OpenAI chat base URL（可选）"
              value={form.openai}
              onChange={(event) => setForm({ ...form, openai: event.target.value })}
            />
            <Input
              aria-label="OpenAI Responses base URL"
              placeholder="OpenAI responses base URL（可选）"
              value={form.responses}
              onChange={(event) => setForm({ ...form, responses: event.target.value })}
            />
            <Input
              aria-label="密钥环境变量"
              placeholder="密钥环境变量（可选）"
              value={form.keyEnv}
              onChange={(event) => setForm({ ...form, keyEnv: event.target.value })}
            />
            <Input
              aria-label="默认模型"
              placeholder="默认模型（可选）"
              value={form.defaultModel}
              onChange={(event) => setForm({ ...form, defaultModel: event.target.value })}
            />
            <Input.TextArea
              aria-label="模型列表"
              placeholder="模型 id，逗号分隔（可选）"
              rows={3}
              value={form.models}
              onChange={(event) => setForm({ ...form, models: event.target.value })}
            />
            {form.mode === 'edit' && (form.anthropic || form.openai || form.responses) ? (
              <Button loading={busy} onClick={() => void probeModels()}>
                抓取模型列表
              </Button>
            ) : null}
            {formError !== null ? (
              <Alert className={styles.probeError} type="error" showIcon title={formError} />
            ) : null}
          </Space>
        ) : null}
      </Modal>
    </>
  )
}

// ─── Aliases ────────────────────────────────────────────────────────────────

export function AliasesPanel({
  role,
  load = api.providers,
  create = api.aliasCreate,
  update = api.aliasUpdate,
  remove = api.aliasDelete,
}: {
  role: Role | null
  load?: () => Promise<{
    providers: ProviderAdmin[]
    config_providers?: ConfigSeededProvider[]
    model_aliases?: Record<string, ModelAliasEntry>
  }>
  create?: (alias: string, provider: string, upstream?: string) => Promise<unknown>
  update?: (alias: string, provider: string, upstream?: string) => Promise<unknown>
  remove?: (alias: string) => Promise<unknown>
}) {
  const [providers, setProviders] = useState<ProviderAdmin[]>([])
  const [configProviders, setConfigProviders] = useState<ConfigSeededProvider[]>([])
  const [aliases, setAliases] = useState<Record<string, ModelAliasEntry>>({})
  const [error, setError] = useState<string | null>(null)
  const [action, setAction] = useState<{ ok: boolean; text: string } | null>(null)
  const [form, setForm] = useState({ alias: '', provider: '', upstream: '' })
  const [mode, setMode] = useState<'create' | 'edit'>('create')
  const [editing, setEditing] = useState<string | null>(null)
  const [pendingDelete, setPendingDelete] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const writable = canManageProviders(role)

  async function reload(): Promise<void> {
    try {
      const response = await load()
      setProviders(response.providers)
      setConfigProviders(response.config_providers ?? [])
      setAliases(response.model_aliases ?? {})
      setError(null)
    } catch (failure) {
      setError(errorText(failure))
    }
  }
  useEffect(() => {
    void reload()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load])

  function startCreate(): void {
    setMode('create')
    setEditing(null)
    setForm({ alias: '', provider: '', upstream: '' })
  }

  function startEdit(alias: string, entry: ModelAliasEntry): void {
    setMode('edit')
    setEditing(alias)
    setForm({
      alias,
      provider: entry.provider,
      upstream: entry.upstream_model ?? '',
    })
  }

  async function save(): Promise<void> {
    if (busy) return
    if (form.alias.trim() === '' || form.provider === '') {
      setAction({ ok: false, text: '别名与 provider 必填' })
      return
    }
    setBusy(true)
    try {
      if (mode === 'edit' && editing !== null) {
        await update(editing, form.provider, form.upstream.trim() || undefined)
        setAction({ ok: true, text: `别名 ${editing} 已更新` })
      } else {
        await create(form.alias.trim(), form.provider, form.upstream.trim() || undefined)
        setAction({ ok: true, text: '别名已创建' })
      }
      startCreate()
      await reload()
    } catch (failure) {
      setAction({ ok: false, text: errorText(failure) })
    } finally {
      setBusy(false)
    }
  }

  async function removeAlias(alias: string): Promise<void> {
    setBusy(true)
    try {
      await remove(alias)
      setAction({ ok: true, text: '别名已删除' })
      setPendingDelete(null)
      if (editing === alias) startCreate()
      await reload()
    } catch (failure) {
      setAction({ ok: false, text: errorText(failure) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <SectionHead title="别名" hint="别名短名可直接作为模型值使用。" />
      {error !== null ? <LoadError message={error} /> : null}
      {action !== null ? (
        <Alert type={action.ok ? 'success' : 'error'} showIcon title={action.text} />
      ) : null}
      {writable ? (
        <Space wrap className={styles.form}>
          <Input
            aria-label="别名"
            placeholder="别名"
            disabled={mode === 'edit'}
            value={form.alias}
            onChange={(e) => setForm({ ...form, alias: e.target.value })}
          />
          <Select
            aria-label="别名 provider"
            value={form.provider || undefined}
            placeholder="provider"
            options={[
              ...providers.map((provider) => ({ value: provider.name, label: provider.name })),
              // config.toml 种子行同样可作为别名目标；标注来源，避免与
              // store 行混为一谈（qa-round14 D-4-2）。
              ...configProviders
                .filter((provider) => !providers.some((row) => row.name === provider.name))
                .map((provider) => ({
                  value: provider.name,
                  label: `${provider.name}（config 种子）`,
                })),
            ]}
            onChange={(provider) => setForm({ ...form, provider })}
          />
          <Input aria-label="上游模型" placeholder="上游模型（可选）" value={form.upstream} onChange={(e) => setForm({ ...form, upstream: e.target.value })} />
          <Button loading={busy} onClick={() => void save()}>
            {mode === 'edit' ? '保存' : '新建别名'}
          </Button>
          {mode === 'edit' ? (
            <Button onClick={startCreate}>取消编辑</Button>
          ) : null}
        </Space>
      ) : null}
      {Object.keys(aliases).length === 0 ? <Empty description="还没有别名" /> : null}
      {Object.entries(aliases).map(([alias, entry]) => (
        <div key={alias} className={styles.row}>
          <div>
            <strong>{alias}</strong>
            <span className={styles.meta}>
              {entry.provider}
              {entry.upstream_model ? ` → ${entry.upstream_model}` : ''}
            </span>
          </div>
          {writable ? (
            <Space size={4}>
              <Button
                size="small"
                aria-label={`编辑别名 ${alias}`}
                onClick={() => startEdit(alias, entry)}
              >
                编辑
              </Button>
              <Button size="small" danger loading={busy} onClick={() => setPendingDelete(alias)}>
                删除
              </Button>
            </Space>
          ) : null}
        </div>
      ))}

      <Modal
        open={pendingDelete !== null}
        title="删除别名"
        okText="删除"
        okButtonProps={{ danger: true }}
        cancelText="取消"
        confirmLoading={busy}
        onCancel={() => setPendingDelete(null)}
        onOk={() => {
          if (pendingDelete !== null) void removeAlias(pendingDelete)
        }}
      >
        <p>
          删除别名「{pendingDelete}」？使用该短名的会话将回退到原模型值。
        </p>
      </Modal>
    </>
  )
}

// ─── Agents ─────────────────────────────────────────────────────────────────

const INSTALL_RECIPES = [
  { name: 'Claude Code', recipe: 'claude-code' },
  { name: 'Codex', recipe: 'codex' },
  { name: 'OpenCode', recipe: 'opencode' },
] as const

const AGENT_SHAPES = [
  { value: 'claude', label: 'Claude（claude 驱动）' },
  { value: 'opencode', label: 'OpenCode（ACP 预设）' },
  { value: 'custom', label: '自定义 ACP 命令' },
] as const

interface AgentForm {
  mode: 'create' | 'edit'
  id: string
  display: string
  shape: 'keep' | 'claude' | 'opencode' | 'custom'
  driver: string
  path: string
  command: string
  args: string
  workDir: string
  sessionsDir: string
  slots: Record<ModelSlot, string>
  defaultSlot: ModelSlot
}

type SlotState = Record<ModelSlot, string>

function emptySlots(): SlotState {
  return { fable: '', opus: '', sonnet: '', haiku: '' }
}

function slotsFromMapping(mapping: ModelSlotMapping | undefined): {
  slots: SlotState
  defaultSlot: ModelSlot
} {
  const slots = emptySlots()
  if (mapping) {
    for (const slot of MODEL_SLOTS) {
      if (mapping[slot]) slots[slot] = mapping[slot]!
    }
  }
  const enabled = MODEL_SLOTS.filter((slot) => slots[slot] !== '')
  return {
    slots,
    defaultSlot: mapping?.default ?? enabled[0] ?? 'sonnet',
  }
}

function isClaudeForm(form: AgentForm): boolean {
  return form.shape === 'claude' || (form.shape === 'keep' && form.driver === 'claude')
}

/** Empty draft slot state is "not configured": leave the stored mapping alone. */
function slotMappingFor(form: AgentForm): ModelSlotMapping | null {
  const enabled = MODEL_SLOTS.filter((slot) => form.slots[slot] !== '')
  if (enabled.length === 0) return null
  const mapping: ModelSlotMapping = { default: form.defaultSlot }
  for (const slot of enabled) mapping[slot] = form.slots[slot]
  return mapping
}

function agentPayload(form: AgentForm): Partial<AgentLaunchPayload> {
  const display = form.display.trim() || null
  const args = form.args.trim() ? form.args.trim().split(/\s+/) : null
  const workDir = form.workDir.trim() || null
  const sessionsDir = form.sessionsDir.trim() || null
  if (form.mode === 'edit' && form.shape === 'keep') {
    return { display, args, work_dir: workDir, sessions_dir: sessionsDir }
  }
  if (form.shape === 'claude') {
    return {
      driver: 'claude',
      path: form.path.trim() || 'claude',
      args,
      display,
      work_dir: workDir,
      sessions_dir: sessionsDir,
    }
  }
  if (form.shape === 'opencode') {
    return {
      driver: 'acp',
      path: 'opencode',
      args: ['acp', ...(args ?? [])],
      display,
      work_dir: workDir,
    }
  }
  const argv = form.command.trim().split(/\s+/).filter(Boolean)
  return {
    driver: 'acp',
    path: argv[0] ?? '',
    args: [...argv.slice(1), ...(args ?? [])],
    display,
    work_dir: workDir,
  }
}

/** Agent directory with create/edit/delete, install recipes and builtin row. */
export function AgentsPanel({
  role,
  load = api.agents,
  loadProviders = api.providers,
  install = api.agentInstall,
  remove = api.agentsDelete,
  create = api.agentsCreate,
  update = api.agentsUpdate,
}: {
  role: Role | null
  load?: () => Promise<{ agents: AgentKindInfo[] }>
  loadProviders?: () => Promise<{ providers: ProviderAdmin[] }>
  install?: (recipe: string) => Promise<unknown>
  remove?: (id: string) => Promise<unknown>
  create?: (id: string, agent: AgentLaunchPayload) => Promise<unknown>
  update?: (id: string, patch: Partial<AgentLaunchPayload>) => Promise<unknown>
}) {
  const [agents, setAgents] = useState<AgentKindInfo[] | null>(null)
  const [modelOptions, setModelOptions] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)
  const [action, setAction] = useState<{ ok: boolean; text: string } | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [form, setForm] = useState<AgentForm | null>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const writable = canManageAgents(role)

  async function reload(): Promise<void> {
    try {
      setAgents((await load()).agents)
      setError(null)
    } catch (failure) {
      setAgents([])
      setError(errorText(failure))
    }
  }
  useEffect(() => {
    void reload()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load])

  useEffect(() => {
    let cancelled = false
    loadProviders()
      .then((response) => {
        if (cancelled) return
        const ids = new Set<string>()
        for (const provider of response.providers) {
          for (const model of provider.models ?? []) ids.add(model.id)
        }
        setModelOptions([...ids])
      })
      .catch(() => {
        /* catalog is optional: the slot selectors stay empty */
      })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadProviders])

  async function run(key: string, text: string, fn: () => Promise<unknown>): Promise<void> {
    setBusy(key)
    setAction(null)
    try {
      await fn()
      setAction({ ok: true, text })
      await reload()
    } catch (failure) {
      setAction({ ok: false, text: errorText(failure) })
    } finally {
      setBusy(null)
    }
  }

  const builtin = agents?.find((agent) => agent.id === 'native') ?? null
  const managed = (agents ?? []).filter((agent) => agent.id !== 'native')
  const catalogIds = new Set(managed.map((agent) => agent.id))
  const recipes = INSTALL_RECIPES.filter(
    (recipe) =>
      !catalogIds.has(recipe.recipe) &&
      !(agents ?? []).some(
        (agent) => agent.reachable && (agent.path_raw ?? '').includes(`agent-tools/${recipe.recipe}`),
      ),
  )

  function openCreate(): void {
    setFormError(null)
    const slots = slotsFromMapping(undefined)
    setForm({
      mode: 'create',
      id: '',
      display: '',
      shape: 'claude',
      driver: 'claude',
      path: 'claude',
      command: 'opencode acp',
      args: '',
      workDir: '',
      sessionsDir: '',
      slots: slots.slots,
      defaultSlot: slots.defaultSlot,
    })
  }

  function openEdit(agent: AgentKindInfo): void {
    setFormError(null)
    const slots = slotsFromMapping(agent.models)
    setForm({
      mode: 'edit',
      id: agent.id,
      display: agent.display_raw ?? (agent.display !== agent.id ? agent.display : ''),
      shape: 'keep',
      driver: agent.driver_raw ?? 'acp',
      path: agent.path_raw ?? 'claude',
      command: 'opencode acp',
      args: (agent.args ?? []).join(' '),
      workDir: agent.work_dir ?? '',
      sessionsDir: agent.sessions_dir ?? '',
      slots: slots.slots,
      defaultSlot: slots.defaultSlot,
    })
  }

  async function save(): Promise<void> {
    if (form === null || busy !== null) return
    const id = form.id.trim()
    if (form.mode === 'create' && id === '') {
      setFormError('agent id 必填')
      return
    }
    if (id === 'native') {
      setFormError('"native" 是内置 agent 保留 id，不可占用')
      return
    }
    if (form.shape === 'custom' && form.command.trim() === '') {
      setFormError('自定义 ACP 需要至少一个命令（argv[0]）')
      return
    }
    const payload = agentPayload(form)
    if (isClaudeForm(form)) {
      const mapping = slotMappingFor(form)
      if (mapping !== null) {
        if (!MODEL_SLOTS.includes(mapping.default) || form.slots[mapping.default] === '') {
          setFormError('默认档位必须落在已启用且有模型的档位上')
          return
        }
        payload.models = mapping
      }
    }
    setBusy('save')
    setFormError(null)
    try {
      if (form.mode === 'create') {
        await create(id, payload as AgentLaunchPayload)
        setAction({ ok: true, text: `已创建 ${id}（免重启生效）` })
      } else {
        await update(id, payload)
        setAction({ ok: true, text: `已更新 ${id}` })
      }
      setForm(null)
      await reload()
    } catch (failure) {
      setFormError(errorText(failure))
    } finally {
      setBusy(null)
    }
  }

  const shapeOptions =
    form?.mode === 'edit'
      ? [{ value: 'keep', label: '保持现有启动定义' }, ...AGENT_SHAPES]
      : [...AGENT_SHAPES]

  return (
    <>
      <SectionHead title="Agent" hint="目录真源；内置 PI agent 不可删除，其余条目增删改免重启。" />
      {error !== null ? <LoadError message={error} /> : null}
      {action !== null ? (
        <Alert type={action.ok ? 'success' : 'error'} showIcon title={action.text} />
      ) : null}
      <Space wrap>
        <Button loading={busy === 'refresh'} onClick={() => void run('refresh', '已刷新', reload)}>
          刷新
        </Button>
        {writable ? <Button onClick={openCreate}>新建 agent</Button> : null}
        {writable
          ? recipes.map((recipe) => (
              <Button
                key={recipe.recipe}
                loading={busy === recipe.recipe}
                onClick={() =>
                  void run(recipe.recipe, `${recipe.name} 已安装`, () => install(recipe.recipe))
                }
              >
                安装 {recipe.name}
              </Button>
            ))
          : null}
      </Space>
      {agents === null && error === null ? <Spin /> : null}
      {builtin !== null ? (
        <div className={styles.row}>
          <div>
            <strong>{builtin.id}</strong> <Tag color="blue">内置 · PI</Tag>{' '}
            <Tag color={builtin.reachable ? 'green' : 'default'}>
              {builtin.reachable ? '可达' : (builtin.cause ?? '不可达')}
            </Tag>
            <span className={styles.meta}>内置 agent，不可删除</span>
          </div>
        </div>
      ) : null}
      {agents !== null && managed.length === 0 && recipes.length === 0 ? (
        <Empty description="目录里还没有其它 agent" />
      ) : null}
      {managed.map((agent) => (
        <div key={agent.id} className={styles.row}>
          <div>
            <strong>{agent.id}</strong>
            {agent.display !== agent.id ? <Tag>{agent.display}</Tag> : null}{' '}
            <Tag color={agent.reachable ? 'green' : 'default'}>
              {agent.reachable ? '可达' : '不可达'}
            </Tag>
            <span className={styles.meta}>
              {agent.version ? `${agent.version} · ` : ''}
              {agent.reachable ? (agent.driver_raw ?? '已安装') : (agent.cause ?? '未知原因')}
              {agent.work_dir ? ` · ${agent.work_dir}` : ''}
            </span>
          </div>
          {writable ? (
            <Space size={4}>
              <Button size="small" onClick={() => openEdit(agent)}>
                编辑
              </Button>
              <Button
                size="small"
                danger
                loading={busy === agent.id}
                onClick={() => void run(agent.id, 'Agent 已删除', () => remove(agent.id))}
              >
                删除
              </Button>
            </Space>
          ) : null}
        </div>
      ))}
      {writable
        ? recipes.map((recipe) => (
            <div key={`recipe-${recipe.recipe}`} className={styles.row}>
              <div>
                <strong>{recipe.recipe}</strong> <Tag>未安装</Tag>
                <span className={styles.meta}>{recipe.name} · 安装到私有前缀</span>
              </div>
              <Button
                size="small"
                loading={busy === recipe.recipe}
                onClick={() =>
                  void run(recipe.recipe, `${recipe.name} 已安装`, () => install(recipe.recipe))
                }
              >
                安装
              </Button>
            </div>
          ))
        : null}

      <Modal
        open={form !== null}
        title={form?.mode === 'edit' ? `编辑 agent：${form.id}` : '新建 agent'}
        okText={form?.mode === 'edit' ? '保存' : '创建'}
        cancelText="取消"
        confirmLoading={busy === 'save'}
        onCancel={() => setForm(null)}
        onOk={() => void save()}
      >
        {form !== null ? (
          <Space orientation="vertical" className={styles.form}>
            <Input
              aria-label="agent id"
              placeholder="id（必填，保留 id native 不可用）"
              disabled={form.mode === 'edit'}
              value={form.id}
              onChange={(event) => setForm({ ...form, id: event.target.value })}
            />
            <Input
              aria-label="agent 显示名"
              placeholder="显示名（可选）"
              value={form.display}
              onChange={(event) => setForm({ ...form, display: event.target.value })}
            />
            <Select
              aria-label="启动定义"
              value={form.shape}
              options={shapeOptions.map((option) => ({ ...option }))}
              onChange={(value) =>
                setForm({
                  ...form,
                  shape: value as AgentForm['shape'],
                  driver:
                    value === 'claude' || (value === 'keep' && form.driver === 'claude')
                      ? 'claude'
                      : value === 'keep'
                        ? form.driver
                        : 'acp',
                })
              }
            />
            {form.shape === 'claude' ? (
              <>
                <Input
                  aria-label="claude 路径"
                  placeholder="claude 路径（默认 claude）"
                  value={form.path}
                  onChange={(event) => setForm({ ...form, path: event.target.value })}
                />
                <Input
                  aria-label="sessions 目录"
                  placeholder="sessions 目录（可选）"
                  value={form.sessionsDir}
                  onChange={(event) => setForm({ ...form, sessionsDir: event.target.value })}
                />
              </>
            ) : null}
            {form.shape === 'opencode' ? null : form.shape === 'custom' ? (
              <Input
                aria-label="ACP 命令"
                placeholder="ACP 命令（argv[0] + 参数）"
                value={form.command}
                onChange={(event) => setForm({ ...form, command: event.target.value })}
              />
            ) : null}
            <Input
              aria-label="启动参数"
              placeholder="启动参数（空格分隔，可选）"
              value={form.args}
              onChange={(event) => setForm({ ...form, args: event.target.value })}
            />
            <Input
              aria-label="工作目录"
              placeholder="工作目录（可选）"
              value={form.workDir}
              onChange={(event) => setForm({ ...form, workDir: event.target.value })}
            />
            {isClaudeForm(form) ? (
              <fieldset className={styles.slotGroup} aria-label="模型档位">
                <legend className={styles.slotLegend}>模型档位</legend>
                {MODEL_SLOTS.map((slot) => (
                  <fieldset key={slot} className={styles.slotRow} aria-label={slot}>
                    <legend className={styles.slotName}>{slot}</legend>
                    <label className={styles.slotToggle}>
                      <input
                        type="checkbox"
                        aria-label={`启用 ${slot}`}
                        checked={form.slots[slot] !== ''}
                        onChange={(event) =>
                          setForm({
                            ...form,
                            slots: {
                              ...form.slots,
                              [slot]: event.target.checked ? (modelOptions[0] ?? '') : '',
                            },
                          })
                        }
                      />
                      启用
                    </label>
                    <Select
                      aria-label={`${slot} 模型`}
                      size="small"
                      animation=""
                      transitionName=""
                      value={form.slots[slot] || undefined}
                      placeholder="选择模型"
                      options={modelOptions.map((model) => ({ value: model, label: model }))}
                      onChange={(model) =>
                        setForm({ ...form, slots: { ...form.slots, [slot]: model } })
                      }
                    />
                    <label className={styles.slotToggle}>
                      <input
                        type="radio"
                        name={`agent-default-${form.id}`}
                        aria-label={`${slot} 默认`}
                        disabled={form.slots[slot] === ''}
                        checked={form.defaultSlot === slot}
                        onChange={() => setForm({ ...form, defaultSlot: slot })}
                      />
                      默认
                    </label>
                  </fieldset>
                ))}
              </fieldset>
            ) : null}
            {formError !== null ? <Alert type="error" showIcon title={formError} /> : null}
          </Space>
        ) : null}
      </Modal>
    </>
  )
}

// ─── Skills ─────────────────────────────────────────────────────────────────

export function SkillsPanel({
  role,
  load = api.skillsList,
  detail = api.skillDetail,
  sync = api.skillsSync,
  remove = api.skillsDelete,
}: {
  role: Role | null
  load?: () => Promise<{ skills: SkillEntry[] }>
  detail?: (name: string) => Promise<SkillDetail>
  sync?: () => Promise<SkillsSyncResponse>
  remove?: (name: string) => Promise<unknown>
}) {
  const [skills, setSkills] = useState<SkillEntry[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [action, setAction] = useState<{ ok: boolean; text: string } | null>(null)
  const [report, setReport] = useState<SkillsSyncResponse | null>(null)
  const [preview, setPreview] = useState<{ name: string; detail: SkillDetail | null } | null>(null)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const writable = canManageSkills(role)

  async function reload(): Promise<void> {
    try {
      setSkills((await load()).skills)
      setError(null)
    } catch (failure) {
      setError(errorText(failure))
    }
  }
  useEffect(() => {
    void reload()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load])

  function togglePreview(name: string): void {
    if (preview?.name === name) {
      setPreview(null)
      return
    }
    setPreview({ name, detail: null })
    setPreviewError(null)
    detail(name)
      .then((value) => setPreview((current) => (current?.name === name ? { name, detail: value } : current)))
      .catch((failure: unknown) => setPreviewError(errorText(failure)))
  }

  return (
    <>
      <SectionHead title="技能" hint="查看与移除窗口；创建/修改走 CLI 或 git。" />
      <Space>
        <Button loading={busy} onClick={() => void reload()}>
          刷新
        </Button>
        {writable ? (
          <Button
            type="primary"
            loading={busy}
            onClick={() => {
              setBusy(true)
              sync()
                .then((response) => {
                  setReport(response)
                  setAction({ ok: true, text: '同步完成' })
                })
                .catch((failure: unknown) => setAction({ ok: false, text: errorText(failure) }))
                .finally(() => setBusy(false))
            }}
          >
            同步
          </Button>
        ) : null}
      </Space>
      {error !== null ? <LoadError message={error} /> : null}
      {action !== null ? (
        <Alert type={action.ok ? 'success' : 'error'} showIcon title={action.text} />
      ) : null}
      {report !== null ? (
        <Alert
          type="info"
          showIcon
          title={`同步：${report.reports.reduce(
            (sum, entry) => sum + entry.written.length + entry.overwritten.length,
            0,
          )} 写入/覆盖，${report.reports.reduce((sum, entry) => sum + entry.deleted.length, 0)} 删除${
            report.no_placement.length ? `；无落点：${report.no_placement.join(', ')}` : ''
          }`}
        />
      ) : null}
      {skills === null && error === null ? <Spin /> : null}
      {skills?.length === 0 ? (
        <Empty description="技能仓为空。用 sebas skills add、git 或 npx 添加后点刷新。" />
      ) : null}
      {skills?.map((skill) => (
        <div key={skill.name} className={styles.row}>
          <button
            type="button"
            className={styles.skillToggle}
            aria-expanded={preview?.name === skill.name}
            onClick={() => togglePreview(skill.name)}
          >
            <strong>{skill.name}</strong>
          </button>
          <div className={styles.skillBody}>
            <Tag color={skill.valid ? 'green' : 'red'}>{skill.valid ? '有效' : '无效'}</Tag>
            <span className={styles.meta}>
              {skill.description ?? '无描述'}
              {skill.valid ? '' : ` · ${skill.reason ?? '原因未上报'}`}
              {skill.attachments.length ? ` · ${skill.attachments.length} 个附件` : ''}
            </span>
            {preview?.name === skill.name ? (
              <div className={styles.skillPreview}>
                {previewError !== null ? <Alert type="error" showIcon title={previewError} /> : null}
                {previewError === null && preview.detail === null ? <Spin size="small" /> : null}
                {preview.detail !== null && preview.detail.text === null ? (
                  <Typography.Text type="secondary">SKILL.md 缺失</Typography.Text>
                ) : null}
                {preview.detail !== null && preview.detail.text !== null ? (
                  <>
                    <div
                      className={styles.skillMarkdown}
                      dangerouslySetInnerHTML={{
                        __html: renderMarkdown(stripFrontmatter(preview.detail.text)),
                      }}
                    />
                    {preview.detail.attachments.length > 0 ? (
                      <ul className={styles.skillFiles}>
                        {preview.detail.attachments.map((file) => (
                          <li key={file}>{file}</li>
                        ))}
                      </ul>
                    ) : null}
                  </>
                ) : null}
              </div>
            ) : null}
          </div>
          {writable ? (
            <Button
              size="small"
              danger
              loading={busy}
              onClick={() => {
                setBusy(true)
                remove(skill.name)
                  .then(async () => {
                    setAction({ ok: true, text: '技能已删除' })
                    await reload()
                  })
                  .catch((failure: unknown) => setAction({ ok: false, text: errorText(failure) }))
                  .finally(() => setBusy(false))
              }}
            >
              删除
            </Button>
          ) : null}
        </div>
      ))}
    </>
  )
}

// ─── Env ────────────────────────────────────────────────────────────────────

export function EnvPanel({ load = api.env }: { load?: () => Promise<{ items: EnvVarEntry[] }> }) {
  const [items, setItems] = useState<EnvVarEntry[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let cancelled = false
    load()
      .then((response) => {
        if (!cancelled) setItems(response.items)
      })
      .catch((failure: unknown) => {
        if (!cancelled) setError(errorText(failure))
      })
    return () => {
      cancelled = true
    }
  }, [load])

  const rows = useMemo(() => items ?? [], [items])
  return (
    <>
      <SectionHead title="环境变量" hint="服务端策划清单；敏感项只显示是否已设置。" />
      {error !== null ? <LoadError message={error} /> : null}
      {items === null && error === null ? <Spin /> : null}
      {items !== null && rows.length === 0 ? <Empty description="没有策划的变量" /> : null}
      {rows.length > 0 ? (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>变量</th>
              <th>用途</th>
              <th>值</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((item) => (
              <tr key={item.name}>
                <td className={styles.mono}>{item.name}</td>
                <td>{item.what}</td>
                <td className={styles.mono}>
                  {item.kind === 'set_unset'
                    ? item.set === undefined
                      ? '无法确定'
                      : item.set
                        ? '已设置（已遮蔽）'
                        : '未设置'
                    : item.value === null
                      ? '未设置（用默认）'
                      : item.value}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </>
  )
}
