// @vitest-environment jsdom
/**
 * Settings modal (IA v3, revamp-settings-nav-and-models-editor +
 * split-env-vars-settings-section + add-webui-multiuser-rbac)：左侧分区导航 +
 * 右侧内容。七个分区（顺序即规约；Users/Services 随角色裁剪可见性）——
 *   - generic    纯通用可配置项分区：语言切换等偏好落地前只呈现说明占位
 *                文案（环境变量表已迁往 env-vars，该分区不再有 env 表）
 *   - appearance 主题三态（system/dark/light，走真实 theme.ts）
 *   - services   watchdog 受管子进程（/api/admin/services 经 adminServicesSafe：
 *                name / desired / actual / uptime + /api/admin/events 最近错误；
 *                im→「飞书 IM」显示映射；无 adapter 时「无 watchdog 控制面」
 *                横幅而非空列表冒充——router 行的 desired/actual/uptime 只在
 *                此分区呈现）。动作按钮随 actual status 互斥（status-driven-
 *                service-rows D2）：running 只 ■、stopped/disabled 只 ▶、
 *                starting/restarting 过渡占位不可点、degraded/failed-startup
 *                ■+⟳、busy 全禁用；core 行纯只读零按钮（D3）；watchdog /
 *                updater 等非受管名不渲染为服务行。停止被拒（400/409 +
 *                active_routed_sessions + count，unify-router-process-shape
 *                D4）→ 二层强制出口对话框：计数 + 流式中断后果，Force stop
 *                以 force 重发、取消不发请求；其余失败仍走既有内联错误
 *   - users      用户管理（add-webui-multiuser-rbac 5.3/5.4，root 专属）：
 *                /api/users 列表 + 新建对话框（用户名/密码/角色下拉）+ 行内
 *                改角色/重置密码/启停/删除；400/409 文案就地展示。分区可见性
 *                随 role 裁剪（Users 仅 root、Services 隐藏于 member/viewer；
 *                role 缺省 = 鉴权关闭的宿主，保持既有分区）
 *   - models     provider 管理列表（/api/providers，条目携带能力
 *                标记；不再渲染 Router 网关卡、不再请求 /api/router）
 *   - env-vars   环境变量只读表（/api/env 服务端策划清单，懒加载）：plain
 *                已设置显实际值、未设置显「未设置（用默认）」、set_unset
 *                敏感项只显已设置/未设置（set 缺失如实显「无法确定」）；
 *                失败 → 分区内联错误态，不渲染空表假象
 *   - about      INSTANCE 段在上（工作区根目录 + 复制、default agent kind
 *                读 /api/about 下发的运行时真值；default provider/model 行
 *                已随 preselect-last-used-model 删除），BUILD 段在下
 *                （/api/about 真实字段）
 * 原 `Settings` 总览分区移除：「全部进程重启」「重置 Settings」两个高危
 * 动作随之删除（任何分区都不得再出现入口）。
 * 缺省首项 generic；上次分区记忆走 localStorage `lastSettingsSection`
 * （缺值/非法值——含旧值 `settings`/`env`——回退 generic）。关闭交互
 * （按钮 / Esc / 遮罩）一并覆盖；子 `<wa-select>` 冒泡的 `wa-hide` 不得
 * 连带关闭对话框（来源守卫）。
 * fetch 交互（revamp-settings-nav-and-models-editor，取代 add-fetch-models
 * 的行内 🔍 + 结果列表挑选流）：入口在编辑器「Models」区块标题旁（仅编辑
 * 既有 provider 且有可用 base URL 时渲染）；成功整单替换草稿列表（按 id
 * 去重、同 id 保留人工 tags），保存与否走普通编辑流；失败保留草稿并内联
 * 呈现净化原因。
 * api client 全量 mock（含前序 agent 留下的 admin stubs，本轮沿用）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ---- WA 渲染垫片（共享）--------------------------------------------------
// 本文件渲染 WA 表单/弹窗组件：jsdom 的 ElementInternals 缺 setValidity、
// HTMLDialogElement 缺 showModal、Element 缺 getAnimations，都会抛未处理
// rejection。共享实现见 test-support/wa-polyfills.ts。
import { installWaDomPolyfills } from '../test-support/wa-polyfills.js'

installWaDomPolyfills()

// jsdom 这里不提供 localStorage（about:blank origin），沿用仓库的内存
// polyfill 约定（见 transcript-view.test.ts）；Appearance 分区的主题
// 持久化走真实 theme.ts，分区记忆走 `lastSettingsSection`，都需要它真实可读写。
const themeStore = new Map<string, string>()
const themeLs = {
  getItem: (k: string) => themeStore.get(k) ?? null,
  setItem: (k: string, v: string) => void themeStore.set(k, v),
  removeItem: (k: string) => void themeStore.delete(k),
  clear: () => themeStore.clear(),
  key: () => null,
  get length() {
    return themeStore.size
  },
}
Object.defineProperty(globalThis, 'localStorage', { value: themeLs, configurable: true })
beforeEach(() => themeStore.clear())

const apiMocks = vi.hoisted(() => ({
  router: vi.fn(),
  about: vi.fn(),
  env: vi.fn(),
  providers: vi.fn(),
  providerPresets: vi.fn(),
  providerCreate: vi.fn(),
  providerUpdate: vi.fn(),
  providerDelete: vi.fn(),
  fetchProviderModels: vi.fn(),
  adminServices: vi.fn(),
  adminEvents: vi.fn(),
  adminServicesSafe: vi.fn(),
  adminEventsSafe: vi.fn(),
  enableService: vi.fn(),
  disableService: vi.fn(),
  restartService: vi.fn(),
  fsBrowseDirs: vi.fn(),
  usersList: vi.fn(),
  authMe: vi.fn(),
  usersCreate: vi.fn(),
  usersSetPassword: vi.fn(),
  usersSetRole: vi.fn(),
  usersSetEnabled: vi.fn(),
  usersDelete: vi.fn(),
  agents: vi.fn(),
  agentsCreate: vi.fn(),
  agentsUpdate: vi.fn(),
  agentsDelete: vi.fn(),
  skillsList: vi.fn(),
  skillDetail: vi.fn(),
  skillsDelete: vi.fn(),
  skillsSync: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
  // 与真 ApiError 同形（client.ts）：status + 机器可读拒绝码 code + 数值
  // 载荷 count（unify-router-process-shape 2.3 的 400 拒绝经此携带）。
  ApiError: class ApiError extends Error {
    readonly status: number
    readonly code: string | null
    readonly count: number | null
    constructor(
      status: number,
      message: string,
      code: string | null = null,
      count: number | null = null,
    ) {
      super(message)
      this.status = status
      this.code = code
      this.count = count
    }
  },
  api: {
    router: apiMocks.router,
    about: apiMocks.about,
    env: apiMocks.env,
    providers: apiMocks.providers,
    providerPresets: apiMocks.providerPresets,
    providerCreate: apiMocks.providerCreate,
    providerUpdate: apiMocks.providerUpdate,
    providerDelete: apiMocks.providerDelete,
    fetchProviderModels: apiMocks.fetchProviderModels,
    adminServices: apiMocks.adminServices,
    adminEvents: apiMocks.adminEvents,
    adminServicesSafe: apiMocks.adminServicesSafe,
    adminEventsSafe: apiMocks.adminEventsSafe,
    enableService: apiMocks.enableService,
    disableService: apiMocks.disableService,
    restartService: apiMocks.restartService,
    fsBrowseDirs: apiMocks.fsBrowseDirs,
    usersList: apiMocks.usersList,
    authMe: apiMocks.authMe,
    usersCreate: apiMocks.usersCreate,
    usersSetPassword: apiMocks.usersSetPassword,
    usersSetRole: apiMocks.usersSetRole,
    usersSetEnabled: apiMocks.usersSetEnabled,
    usersDelete: apiMocks.usersDelete,
    agents: apiMocks.agents,
    agentsCreate: apiMocks.agentsCreate,
    agentsUpdate: apiMocks.agentsUpdate,
    agentsDelete: apiMocks.agentsDelete,
    skillsList: apiMocks.skillsList,
    skillDetail: apiMocks.skillDetail,
    skillsDelete: apiMocks.skillsDelete,
    skillsSync: apiMocks.skillsSync,
  },
  // 角色词表（渲染层常量，Users 分区的角色下拉数据源）与真模块同值。
  ROLES: ['root', 'admin', 'member', 'viewer'] as const,
  // 呈现用错误文本（webui-i18n-sweep 1.4）：与真实现同语义（Error → message）。
  errorText: (err: unknown) => (err instanceof Error ? err.message : String(err)),
}))

import './settings-modal.js'
import type { SebasSettingsModal } from './settings-modal.js'
import { SebasSettingsModal as SettingsModalImpl } from './settings-modal.js'
// mocked 模块里的 ApiError 类——与组件内 instanceof 同一构造器。
import { ApiError } from '../api/client.js'
import { applyThemeMode } from '../theme.js'

async function mount(open = true): Promise<SebasSettingsModal> {
  const el = document.createElement('sebas-settings-modal') as SebasSettingsModal
  el.open = open
  document.body.appendChild(el)
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

/** Drain the microtask queue so child-component fetches settle. */
async function settle(el: SebasSettingsModal): Promise<void> {
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
}

function navItems(el: SebasSettingsModal): HTMLElement[] {
  return [...el.shadowRoot!.querySelectorAll<HTMLElement>('.nav .nav-item')]
}

/** Click a nav entry by section id and wait for its lazy loads to settle.
 * （fix-webui-qa-round8 6.1）新增「别名」分区后 nav 顺序位移——按 id 寻址，
 * 不再按易碎的数字下标。 */
async function goto(el: SebasSettingsModal, section: string): Promise<void> {
  const labels: Record<string, string> = {
    generic: '通用',
    appearance: '外观',
    services: '服务',
    users: '用户',
    models: '模型',
    aliases: '别名',
    agents: 'Agent',
    skills: '技能',
    'env-vars': '环境变量',
    about: '关于',
  }
  const item = navItems(el).find((b) => b.textContent?.trim() === labels[section])
  if (!item) throw new Error(`nav item not found: ${section}`)
  item.click()
  await el.updateComplete
  await settle(el)
}

function waButtons(el: SebasSettingsModal): HTMLElement[] {
  return [...el.shadowRoot!.querySelectorAll<HTMLElement>('wa-button')]
}

beforeEach(() => {
  // 调用计数跨 test 累积会污染“未调用”断言（如 Services 不得调 /api/router），
  // 先清计数再装实现（clear 只清 calls，不动实现）。
  vi.clearAllMocks()
  apiMocks.agents.mockResolvedValue({
    agents: [
      { id: 'native', display: 'Native Kernel', reachable: true },
      { id: 'claude', display: 'claude', reachable: true },
    ],
  })
  apiMocks.router.mockResolvedValue({
    router: {
      listen: '127.0.0.1:8787',
      provider_count: 2,
      debug: false,
      has_auth: true,
      providers: [],
    },
  })
  apiMocks.providers.mockResolvedValue({
    providers: [
      {
        name: 'alpha',
        preset: 'deepseek',
        base_url_anthropic: 'https://a.example/anthropic',
        base_url_openai_chat: 'https://a.example/v1',
        base_url_openai_responses: null,
        api_key_env: 'DEEPSEEK_API_KEY',
        api_key_configured: true,
        models: [
          { id: 'm1', tags: [] },
          { id: 'm2', tags: ['vision'] },
        ],
      },
      {
        name: 'beta',
        preset: null,
        base_url_anthropic: null,
        base_url_openai_chat: 'https://b.example/v1',
        base_url_openai_responses: null,
        api_key_env: null,
        api_key_configured: false,
        models: [],
      },
    ],
  })
  apiMocks.adminServices.mockResolvedValue({ adapter_ok: true, services: [] })
  apiMocks.adminEvents.mockResolvedValue({ adapter_ok: true, events: [] })
  apiMocks.adminServicesSafe.mockResolvedValue({ adapter_ok: true, services: [] })
  apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
  apiMocks.enableService.mockResolvedValue({ operation_id: 'op-test', status: 'accepted', message: 'accepted' })
  apiMocks.disableService.mockResolvedValue({ operation_id: 'op-test', status: 'accepted', message: 'accepted' })
  apiMocks.restartService.mockResolvedValue({ operation_id: 'op-test', status: 'accepted', message: 'accepted' })
  apiMocks.fsBrowseDirs.mockResolvedValue({ path: '/tmp/test-work', entries: [] })
  // Users 分区（add-webui-multiuser-rbac 5.3）默认空表桩：默认挂载（role
  // 为 null）不渲染该分区、也不发请求；root 挂载的用例在各自 describe 覆写。
  apiMocks.usersList.mockResolvedValue({ users: [] })
  apiMocks.usersCreate.mockResolvedValue({ status: 'ok', username: 'new-user' })
  apiMocks.usersSetPassword.mockResolvedValue({ status: 'ok' })
  apiMocks.usersSetRole.mockResolvedValue({ status: 'ok' })
  apiMocks.usersSetEnabled.mockResolvedValue({ status: 'ok' })
  apiMocks.usersDelete.mockResolvedValue({ status: 'ok' })
  // （fix-webui-qa-findings OB2）默认身份 = 非列表内用户：无自指行，
  // 危险控件不因 self 误禁；自指用例在各自 describe 覆写。
  apiMocks.authMe.mockResolvedValue({ enabled: true, authenticated: true, username: 'someone-else' })
  // Skills 分区（add-agent-skills 5.2）默认桩：两个有效条目 + 一个 invalid。
  apiMocks.skillsList.mockResolvedValue({
    skills: [
      {
        name: 'beads',
        description: 'beads 工作流',
        attachments: [],
        valid: true,
      },
      {
        name: 'my-deploy',
        description: '部署脚本',
        attachments: ['ref.md', 'scripts/deploy.sh'],
        valid: true,
      },
      {
        name: 'broken',
        description: null,
        attachments: [],
        valid: false,
        reason: 'SKILL.md 必须以 --- 围栏开头（frontmatter 缺失）',
      },
    ],
  })
  apiMocks.skillDetail.mockResolvedValue({
    name: 'beads',
    text: '---\nname: beads\ndescription: beads 工作流\n---\n\n# beads\n\nRun `bd ready` first.',
    attachments: [],
  })
  apiMocks.skillsDelete.mockResolvedValue({ status: 'deleted', name: 'beads' })
  apiMocks.skillsSync.mockResolvedValue({
    reports: [
      {
        backend: 'claude',
        written: ['beads'],
        overwritten: ['grill-me'],
        deleted: ['old-skill'],
        private_ignored: 2,
      },
    ],
    no_placement: ['gemini'],
  })
  apiMocks.providerPresets.mockResolvedValue({
    presets: [
      {
        name: 'deepseek',
        base_url_anthropic: 'https://api.deepseek.com/anthropic',
        base_url_openai_chat: 'https://api.deepseek.com',
        base_url_openai_responses: null,
        api_key_env: 'DEEPSEEK_API_KEY',
        models: [{ id: 'deepseek-chat', tags: [] }],
      },
    ],
  })
  apiMocks.about.mockResolvedValue({
    uptime: '3h 12m',
    version: '0.4.2',
    build_time: '2026-09-29 08:30',
    git_branch: 'main',
    git_hash: 'abc1234',
    rustc: { state: 'ok', version: 'rustc 1.88.0 (hash)' },
    rustc_required: '1.90',
    router_listen: '127.0.0.1:8787',
    provider_count: 2,
    default_agent_kind: 'claude',
  })
  // /api/env 策划清单默认桩：四条覆盖三种呈现形态（plain 已设置/未设置、
  // set_unset 已设置/未设置）。
  apiMocks.env.mockResolvedValue({
    items: [
      {
        name: 'SEBAS_STATE_DIR',
        what: 'State directory: every state file/DB derives from it',
        kind: 'plain',
        value: '/tmp/sebas-itest',
      },
      {
        name: 'SEBAS_HANG_TIMEOUT_SECS',
        what: 'Agent driver hang timeout (seconds)',
        kind: 'plain',
        value: null,
      },
      {
        name: 'SEBAS_CONTROL_SECRET',
        what: 'Watchdog control plane secret',
        kind: 'set_unset',
        value: null,
        set: true,
      },
      {
        name: 'SEBAS_FEISHU_APP_SECRET',
        what: 'Feishu app secret',
        kind: 'set_unset',
        value: null,
        set: false,
      },
    ],
  })
})

afterEach(() => {
  document.body.innerHTML = ''
  localStorage.removeItem('sebas:theme')
  localStorage.removeItem('lastSettingsSection')
  document.documentElement.classList.remove('wa-dark')
})

describe('sebas-settings-modal sections', () => {
  // （fix-webui-qa-round9 4.4，agent-workbench）设置弹窗的可访问对话框语义：
  // 容器以 dialog 呈现（role + aria-modal + 可访问名），分区内容在可访问性
  // 树中可及（nav 有自己的 aria-label）。
  it('the panel carries dialog semantics with an accessible name (round9 4.4)', async () => {
    const el = await mount()
    const panel = el.shadowRoot!.querySelector<HTMLElement>('.panel')!
    expect(panel).toBeTruthy()
    expect(panel.getAttribute('role')).toBe('dialog')
    expect(panel.getAttribute('aria-modal')).toBe('true')
    const name = panel.getAttribute('aria-label') ?? ''
    expect(name.trim().length).toBeGreaterThan(0)
    expect(name).toContain('设置')
    // 分区进入可访问性树：导航有可访问名、内容区在 dialog 内。
    expect(panel.querySelector('.nav')?.getAttribute('aria-label')).toBeTruthy()
    expect(panel.querySelector('.content')).toBeTruthy()
    el.remove()
  })

  it('renders the left nav with exactly Generic/Appearance/Services/Models/Skills/Env Vars/About', async () => {
    const el = await mount()
    const labels = navItems(el).map((b) => b.textContent?.trim())
    expect(labels).toEqual([
      '通用',
      '外观',
      '服务',
      '模型',
      // （fix-webui-qa-round8 6.1）别名分区紧跟模型。
      '别名',
      'Agent',
      '技能',
      '环境变量',
      '关于',
    ])
    el.remove()
  })

  it('separates the nav into groups: a break before Services and a tail break pinning the Env Vars · About bottom group', async () => {
    const el = await mount()
    const nav = el.shadowRoot!.querySelector('.nav')!
    const seps = [...nav.querySelectorAll<HTMLElement>('.nav-sep')]
    expect(seps.length).toBe(2)
    // 底部组上方的分隔线带 .tail（margin-top: auto 压底），且紧贴 Env Vars 项。
    const tail = seps.find((s) => s.classList.contains('tail'))!
    expect(tail).toBeTruthy()
    expect(tail.previousElementSibling?.textContent?.trim()).toBe('技能')
    expect(tail.nextElementSibling?.classList.contains('nav-item')).toBe(true)
    expect(tail.nextElementSibling?.textContent?.trim()).toBe('环境变量')
    // 底部组内 Env Vars → About 之间不再有分隔线（同组并列）。
    expect(tail.nextElementSibling?.nextElementSibling?.textContent?.trim()).toBe('关于')
    // 另一条在 Services 项之前（appearance|services 组间线）。
    const plain = seps.find((s) => !s.classList.contains('tail'))!
    expect(plain.nextElementSibling?.textContent?.trim()).toBe('服务')
    expect(plain.previousElementSibling?.textContent?.trim()).toBe('外观')
    el.remove()
  })

  it('defaults to the Generic section — preferences placeholder, no env table, no /api/env call', async () => {
    const el = await mount()
    expect(el.section).toBe('generic')
    await settle(el)
    // split-env-vars-settings-section：Generic 收敛为纯偏好分区，env 表迁出。
    expect(el.shadowRoot!.querySelector('.env-table')).toBeNull()
    expect(apiMocks.env).not.toHaveBeenCalled()
    // 占位文案指明偏好项（语言切换等）后续提供（textContent 含模板换行，
    // 先折叠空白再断言）。
    const text = (el.shadowRoot!.textContent ?? '').replace(/\s+/g, ' ')
    expect(text).toContain('以后会在这里提供')
    el.remove()
  })

  it('renders no maintenance actions anywhere (restart-all and reset retired)', async () => {
    const el = await mount()
    for (const section of ['generic', 'appearance', 'services', 'models', 'aliases', 'agents', 'skills', 'env-vars', 'about']) {
      await goto(el, section)
      const buttons = waButtons(el).map((b) => b.textContent?.trim())
      expect(buttons).not.toContain('全部进程重启')
      expect(buttons).not.toContain('重置 Settings')
      expect(el.shadowRoot!.querySelector('.danger-zone')).toBeNull()
    }
    el.remove()
  })

  it('remembers lastSettingsSection; legacy settings/env values fall back to generic', async () => {
    localStorage.setItem('lastSettingsSection', 'services')
    const el = await mount()
    await settle(el)
    expect(el.section).toBe('services')
    // 打开期间切换分区会写回记忆。
    await goto(el, "models")
    expect(localStorage.getItem('lastSettingsSection')).toBe('models')
    el.remove()

    // 本变更前的合法分区名（Settings 总览、Env）现在都是非法值 → 回退缺省。
    for (const legacy of ['settings', 'env', 'nope']) {
      localStorage.setItem('lastSettingsSection', legacy)
      const el2 = await mount()
      await settle(el2)
      expect(el2.section).toBe('generic')
      el2.remove()
    }
  })

  it('Services section renders rows from /api/admin/services with im→飞书 IM mapping', async () => {
    apiMocks.router.mockClear()
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [
        { name: 'im', status: 'running', desired: 'running', uptime_secs: 3725 },
        { name: 'router', status: 'stopped', desired: 'stopped', uptime_secs: null },
      ],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({
      adapter_ok: true,
      events: [{ seq: 1, operation_id: 'op-1', kind: 'service_error', message: 'im worker boom' }],
    })
    const el = await mount()
    await goto(el, "services")
    expect(el.section).toBe('services')
    // 真源是 adminServicesSafe；renderServices 不得再读 /api/router。
    expect(apiMocks.adminServicesSafe).toHaveBeenCalled()
    expect(apiMocks.adminEventsSafe).toHaveBeenCalled()
    expect(apiMocks.router).not.toHaveBeenCalled()
    const cards = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')]
    expect(cards.length).toBe(2)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('飞书 IM')
    // 内部名锚点保留，供与 /api/admin/services 对账。
    const ids = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-id')].map((s) =>
      s.textContent?.trim(),
    )
    expect(ids).toEqual(['im', 'router'])
    expect(text).toContain('期望 running · 状态 running · 已运行 1h 2m')
    expect(text).toContain('最近错误')
    expect(text).toContain('im worker boom')
    // 动作按钮随 actual status 互斥（status-driven-service-rows）：im running
    // 只显 ■，router stopped 只显 ▶。
    const enables = [...el.shadowRoot!.querySelectorAll('button[title="启用服务"]')]
    expect(enables.length).toBe(1)
    const disables = [...el.shadowRoot!.querySelectorAll('button[title="停用服务"]')]
    expect(disables.length).toBe(1)
    el.remove()
  })

  it('core row renders no action buttons at all — read-only (restart included)', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [
        { name: 'core', status: 'running', desired: 'running', uptime_secs: 60 },
        { name: 'router', status: 'stopped', desired: 'stopped', uptime_secs: null },
      ],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
    const el = await mount()
    await goto(el, "services")
    const cards = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')]
    const coreCard = cards.find((c) => c.querySelector('.service-id')?.textContent === 'core')!
    expect(coreCard).toBeTruthy()
    // core 纯只读（status-driven-service-rows D3）：无 ▶ / ■ / ⟳ 任何按钮。
    expect(coreCard.querySelector('button[title="启用服务"]')).toBeNull()
    expect(coreCard.querySelector('button[title="停用服务"]')).toBeNull()
    expect(coreCard.querySelector('button[title="重启服务"]')).toBeNull()
    expect(coreCard.querySelectorAll('button').length).toBe(0)
    // 动作区容器仍渲染（定宽占位，D4）。
    expect(coreCard.querySelector('.service-actions')).not.toBeNull()
    el.remove()
  })

  it('renders no watchdog/updater rows — only real managed services', async () => {
    // 合成行已在后端删除（status-driven-service-rows 1.1）；前端如实渲染
    // /api/admin/services 所给行，且永不自行补 watchdog / updater 行。
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [
        { name: 'core', status: 'running', desired: 'running', uptime_secs: 1 },
        { name: 'webui', status: 'running', desired: 'running', uptime_secs: 2 },
      ],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
    const el = await mount()
    await goto(el, "services")
    const ids = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-id')].map((s) =>
      s.textContent?.trim(),
    )
    expect(ids).toEqual(['core', 'webui'])
    expect(ids).not.toContain('watchdog')
    expect(ids).not.toContain('updater')
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).not.toContain('watchdog')
    expect(text).not.toContain('updater')
    el.remove()
  })

  it('renders only managed names even if the backend leaks a synthetic entry', async () => {
    // 纵深防御：非受管名（watchdog/updater/feishu）不得渲染为服务行
    // （spec「名称不属于受管集合的条目 SHALL NOT 渲染为服务行」）。
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [
        { name: 'router', status: 'running', desired: 'running', uptime_secs: 3 },
        { name: 'watchdog', status: 'running', desired: 'enabled', uptime_secs: null },
        { name: 'updater', status: 'idle', desired: 'enabled', uptime_secs: null },
        { name: 'feishu', status: 'running', desired: 'running', uptime_secs: null },
      ],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
    const el = await mount()
    await goto(el, "services")
    const ids = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-id')].map((s) =>
      s.textContent?.trim(),
    )
    expect(ids).toEqual(['router'])
    el.remove()
  })

  it('Services section shows the no-adapter banner without rows or actions', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({ adapter_ok: false, services: [] })
    const el = await mount()
    await goto(el, "services")
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('无 watchdog 控制面')
    expect(el.shadowRoot!.querySelectorAll('.service-card').length).toBe(0)
    expect(el.shadowRoot!.querySelectorAll('.service-actions button').length).toBe(0)
    el.remove()
  })

  it('Services section is the only place that renders router desired/actual/uptime (4.1)', async () => {
    // 4.1：router 行在 Services 呈现 desired / actual / uptime；Models 分区
    // 不出现任何 router 运行状态。adapter_ok:false 时横幅呈现而非空列表冒充
    // （专属用例见下）。
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [{ name: 'router', status: 'running', desired: 'running', uptime_secs: 95 }],
    })
    const el = await mount()
    await goto(el, "services")
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('router')
    expect(text).toContain('期望 running · 状态 running · 已运行 1m')
    el.remove()
  })

  it('Models section renders the provider list only — no gateway card, no /api/router, no row-level fetch', async () => {
    const el = await mount()
    apiMocks.router.mockClear()
    await goto(el, "models")
    expect(el.section).toBe('models')
    // 3.4：Models 渲染不再发起 /api/router 请求，也不出现网关卡。
    expect(apiMocks.router).not.toHaveBeenCalled()
    expect(apiMocks.providers).toHaveBeenCalled()
    expect(apiMocks.providerPresets).toHaveBeenCalled()
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).not.toContain('Router 路由网关')
    expect(text).not.toContain('127.0.0.1:8787')
    const rows = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.provider-row')]
    expect(rows.length).toBe(2)
    expect(text).toContain('alpha')
    expect(text).toContain('deepseek · 内置')
    expect(text).toContain('自定义')
    expect(text).toContain('https://a.example/anthropic')
    expect(text).toContain('beta')
    // 列表呈现模型条目与能力标记（条目 = id + tags；text 隐含）。
    const chips = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.model-chip')]
    expect(chips.map((c) => (c.textContent ?? '').replace(/\s+/g, ''))).toContain('m2vision')
    // revamp…4.1：provider 行不再带 🔍（入口移进编辑器）。
    expect(el.shadowRoot!.querySelector('button[data-testid="fetch-models"]')).toBeNull()
    el.remove()
  })

  it('About section shows INSTANCE (overview items) above BUILD (/api/about)', async () => {
    const el = await mount()
    await goto(el, "about")
    expect(el.section).toBe('about')
    expect(apiMocks.about).toHaveBeenCalled()
    expect(apiMocks.fsBrowseDirs).toHaveBeenCalled()
    const text = el.shadowRoot!.textContent ?? ''
    // INSTANCE 段：工作区根目录 + default agent kind（读 /api/about 真值）。
    expect(text).toContain('实例')
    expect(text).toContain('工作区根目录')
    expect(text).toContain('/tmp/test-work')
    expect(text).toContain('默认 agent 种类')
    expect(text).toContain('claude')
    // BUILD 段：/api/about 真实字段。
    expect(text).toContain('构建')
    expect(text).toContain('0.4.2')
    expect(text).toContain('3h 12m')
    expect(text).toContain('1.88')
    expect(text).toContain('127.0.0.1:8787')
    // 两段dl分开：instance 在前、build 在后（DOM 顺序即呈现顺序）。
    const lists = [...el.shadowRoot!.querySelectorAll('dl.about-list')]
    expect(lists.length).toBe(2)
    expect(lists[0]!.classList.contains('about-instance')).toBe(true)
    expect(lists[1]!.classList.contains('about-build')).toBe(true)
    // 工作区根目录带复制按钮。
    const copy = el.shadowRoot!.querySelector('button[title="复制工作区根目录"]')
    expect(copy).toBeTruthy()
    el.remove()
  })

  it('About INSTANCE no longer renders the default provider/model row or a Models jump link', async () => {
    const el = await mount()
    await goto(el, "about")
    const text = el.shadowRoot!.textContent ?? ''
    // preselect-last-used-model 3.1：行已删除——无论配置与否都不渲染，
    // 跳转 Models 的链接随之消失（创建预选改 last-used 语义，该行只是
    // 数据源已 404 的假勾选死 UI）。
    expect(text).not.toContain('Default provider / model')
    expect(text).not.toContain('— (set one in Models)')
    expect(
      el.shadowRoot!.querySelector('button[title="Open the Models section"]'),
    ).toBeNull()
    el.remove()
  })

  it('About carries the router-side provider count annotation, distinct from the Models registry (round5 4.3)', async () => {
    const el = await mount()
    await goto(el, "about")
    const text = el.shadowRoot!.textContent ?? ''
    // 口径标注：About 的 Providers 是 router 侧计数（含 debug provider），
    // 与 Models 分区的注册表口径区分，两个数字不一一对应。
    expect(text).toContain('router 侧计数，含 debug provider')
    el.remove()
  })

  it('aria-current tracks the active section', async () => {
    const el = await mount()
    expect(navItems(el)[0]!.getAttribute('aria-current')).toBe('true')
    navItems(el)[1]!.click()
    await el.updateComplete
    expect(navItems(el)[0]!.getAttribute('aria-current')).toBe('false')
    expect(navItems(el)[1]!.getAttribute('aria-current')).toBe('true')
    el.remove()
  })
})

describe('status-driven-service-rows：动作按钮随 actual status 互斥（D2）', () => {
  /** 装一个单行（router）Services 分区，返回 modal 与该行卡片。 */
  async function mountRow(
    status: string,
  ): Promise<{ el: SebasSettingsModal; card: HTMLElement }> {
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [{ name: 'router', status, desired: 'running', uptime_secs: 5 }],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
    const el = await mount()
    await goto(el, "services")
    const card = el.shadowRoot!.querySelector<HTMLElement>('.service-card')!
    expect(card).toBeTruthy()
    return { el, card }
  }

  function buttonsOf(card: HTMLElement): string[] {
    return [...card.querySelectorAll('button')].map((b) => b.textContent?.trim() ?? '')
  }

  function placeholderOf(card: HTMLElement): Element | null {
    return card.querySelector('.service-actions .service-transition')
  }

  it('running 只显 ■（disable），不显 ▶ / ⟳', async () => {
    const { el, card } = await mountRow('running')
    expect(buttonsOf(card)).toEqual(['■'])
    el.remove()
  })

  it('stopped 只显 ▶（enable），不显 ■ / ⟳', async () => {
    const { el, card } = await mountRow('stopped')
    expect(buttonsOf(card)).toEqual(['▶'])
    el.remove()
  })

  it('disabled 只显 ▶（enable）', async () => {
    const { el, card } = await mountRow('disabled')
    expect(buttonsOf(card)).toEqual(['▶'])
    el.remove()
  })

  it.each(['starting', 'restarting'])(
    '%s 渲染不可点的过渡占位（无 ▶ / ■，点击无请求）',
    async (status) => {
      const { el, card } = await mountRow(status)
      expect(buttonsOf(card)).toEqual([])
      const ph = placeholderOf(card)
      expect(ph).not.toBeNull()
      expect(ph!.tagName).toBe('SPAN')
      ph!.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }))
      await el.updateComplete
      expect(apiMocks.enableService).not.toHaveBeenCalled()
      expect(apiMocks.disableService).not.toHaveBeenCalled()
      expect(apiMocks.restartService).not.toHaveBeenCalled()
      el.remove()
    },
  )

  it.each(['degraded', 'failed-startup'])('%s 显 ■ + ⟳，不显 ▶', async (status) => {
    const { el, card } = await mountRow(status)
    expect(buttonsOf(card)).toEqual(['■', '⟳'])
    expect(placeholderOf(card)).toBeNull()
    el.remove()
  })

  it('未知 status 按过渡占位降级（fail-safe，不猜按钮）', async () => {
    const { el, card } = await mountRow('some-future-state')
    expect(buttonsOf(card)).toEqual([])
    expect(placeholderOf(card)).not.toBeNull()
    el.remove()
  })

  it('busy 期间行内动作全部禁用，动作结束后恢复', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [{ name: 'router', status: 'stopped', desired: 'running', uptime_secs: 5 }],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
    let resolveEnable!: (v: unknown) => void
    apiMocks.enableService.mockReturnValue(
      new Promise((resolve) => (resolveEnable = resolve)),
    )
    const el = await mount()
    await goto(el, "services")
    const card = el.shadowRoot!.querySelector<HTMLElement>('.service-card')!
    const enable = card.querySelector<HTMLButtonElement>('button[title="启用服务"]')!
    enable.click()
    await el.updateComplete
    // 执行期间：行内按钮禁用（busy）。
    expect(enable.disabled).toBe(true)
    resolveEnable({ operation_id: 'op-x', status: 'accepted', message: 'accepted' })
    await settle(el)
    expect(enable.disabled).toBe(false)
    // 成功后刷新列表。
    expect(apiMocks.adminServicesSafe).toHaveBeenCalledTimes(2)
    el.remove()
  })

  it('动作区定宽：--service-actions-w 声明一次、.service-actions 引用，每行恰一个定宽容器', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [
        { name: 'core', status: 'running', desired: 'running', uptime_secs: 1 },
        { name: 'router', status: 'running', desired: 'running', uptime_secs: 2 },
        { name: 'webui', status: 'starting', desired: 'running', uptime_secs: null },
      ],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
    const el = await mount()
    await goto(el, "services")
    // 每行（core 只读、running 两钮、过渡占位）都渲染恰一个 .service-actions
    // 定宽容器——jsdom 无布局，真实 x 对齐由 Playwright 冒烟断言。
    const actions = [
      ...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card .service-actions'),
    ]
    expect(actions.length).toBe(3)
    for (const a of actions) {
      expect(a.children.length).toBeGreaterThanOrEqual(0)
    }
    const coreCard = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')].find(
      (c) => c.querySelector('.service-id')?.textContent === 'core',
    )!
    expect(coreCard.querySelector('.service-actions')!.children.length).toBe(0)
    // 自定义属性在组件样式表中声明一次，宽度规则引用 var()。
    const css = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    expect(css.match(/--service-actions-w:/g)?.length).toBe(1)
    expect(css).toMatch(/\.service-card \.service-actions\s*\{[^}]*var\(--service-actions-w\)/)
    el.remove()
  })
})

describe('split-env-vars-settings-section：Env Vars 分区（/api/env）', () => {
  function envRows(el: SebasSettingsModal): HTMLElement[] {
    return [...el.shadowRoot!.querySelectorAll<HTMLElement>('.env-table tbody tr')]
  }

  function valueOf(rows: HTMLElement[], name: string): string | null {
    const row = rows.find((r) => r.querySelector('.var')?.textContent?.trim() === name)
    return row?.querySelector('.value')?.textContent?.trim() ?? null
  }

  it('lazily loads /api/env on first visit and renders the three display states', async () => {
    const el = await mount()
    await settle(el)
    // 懒加载：未访问该分区前不拉取。
    expect(apiMocks.env).not.toHaveBeenCalled()
    await goto(el, "env-vars")
    expect(el.section).toBe('env-vars')
    expect(apiMocks.env).toHaveBeenCalledTimes(1)
    const rows = envRows(el)
    expect(rows.length).toBe(4)
    // plain 已设置 → 实际值。
    expect(valueOf(rows, 'SEBAS_STATE_DIR')).toBe('/tmp/sebas-itest')
    // plain 未设置 → 「未设置（用默认）」。
    expect(valueOf(rows, 'SEBAS_HANG_TIMEOUT_SECS')).toBe('未设置（用默认）')
    // set_unset（敏感）→ 只显已设置/未设置，与 set 布尔一致。
    expect(valueOf(rows, 'SEBAS_CONTROL_SECRET')).toBe('已设置')
    expect(valueOf(rows, 'SEBAS_FEISHU_APP_SECRET')).toBe('未设置')
    el.remove()
  })

  it('never renders a value for set_unset entries even if the response leaks one', async () => {
    // 防御性钉死：遮蔽是服务端责任，但前端对 set_unset 项也不得渲染 value
    // ——合同外的泄漏值不能经前端落到界面。
    apiMocks.env.mockResolvedValue({
      items: [
        {
          name: 'SEBAS_CONTROL_SECRET',
          what: 'Watchdog control plane secret',
          kind: 'set_unset',
          value: 'super-secret-leak',
          set: true,
        },
      ],
    })
    const el = await mount()
    await goto(el, "env-vars")
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).not.toContain('super-secret-leak')
    expect(text).toContain('已设置')
    el.remove()
  })

  it('shows 无法确定 for set_unset entries without a set flag (degraded contract, not fake unset)', async () => {
    // 防御性解析：kind=set_unset 且无 set 字段 → 前端无法断言状态，如实显
    // 「无法确定」，绝不冒充「未设置」。
    apiMocks.env.mockResolvedValue({
      items: [
        {
          name: 'SEBAS_CONTROL_SECRET',
          what: 'Router control-plane secret',
          kind: 'set_unset',
          value: null,
        },
      ],
    })
    const el = await mount()
    await goto(el, "env-vars")
    const rows = envRows(el)
    expect(rows.length).toBe(1)
    expect(rows[0]!.querySelector('.value')?.textContent?.trim()).toBe('无法确定')
    el.remove()
  })

  it('renders an inline error instead of an empty table when /api/env fails', async () => {
    apiMocks.env.mockRejectedValue(new ApiError(500, 'env listing exploded'))
    const el = await mount()
    await goto(el, "env-vars")
    const err = el.shadowRoot!.querySelector('.callout-error[role="alert"]')
    expect(err).toBeTruthy()
    expect(err!.textContent).toContain('env listing exploded')
    // 失败 → 不渲染空表假象。
    expect(el.shadowRoot!.querySelector('.env-table')).toBeNull()
    el.remove()
  })
})

describe('add-agent-skills 5.2：Skills 分区（/api/skills*）', () => {
  /**
   * 导航序（默认 role=null 挂载）：generic(0) appearance(1) services(2)
   * models(3) agents(4) skills(5) env-vars(6) about(7)。本 describe 全用 index 5。
   */
  function skillRows(el: SebasSettingsModal): HTMLElement[] {
    return [...el.shadowRoot!.querySelectorAll<HTMLElement>('[data-testid="skill-row"]')]
  }

  it('lazily loads /api/skills on first visit; rows show name, attachment counts and invalid badge with reason', async () => {
    const el = await mount()
    await settle(el)
    // 懒加载：未访问该分区前不拉取。
    expect(apiMocks.skillsList).not.toHaveBeenCalled()
    await goto(el, "skills")
    expect(el.section).toBe('skills')
    expect(apiMocks.skillsList).toHaveBeenCalledTimes(1)
    const rows = skillRows(el)
    expect(rows.length).toBe(3)
    // 名字 + attachment 数。
    const deploy = rows.find((r) => r.dataset.name === 'my-deploy')!
    expect(deploy.querySelector('.skills-row-atts')?.textContent?.trim()).toBe('2 个附件')
    // invalid 徽标 + 成因（描述位显 reason）。
    const broken = rows.find((r) => r.dataset.name === 'broken')!
    expect(broken.querySelector('[data-testid="skill-invalid"]')).toBeTruthy()
    expect(broken.querySelector('.skills-row-desc')?.textContent).toContain('围栏')
    el.remove()
  })

  it('clicking a row lazily loads the detail and renders sanitized markdown plus attachment list', async () => {
    const el = await mount()
    await goto(el, "skills")
    // 点开前不发详情请求。
    expect(apiMocks.skillDetail).not.toHaveBeenCalled()
    const beads = skillRows(el).find((r) => r.dataset.name === 'beads')!
    beads.querySelector<HTMLElement>('.skills-row-name')!.click()
    await settle(el)
    expect(apiMocks.skillDetail).toHaveBeenCalledWith('beads')
    const preview = el.shadowRoot!.querySelector('[data-testid="skill-preview"]')!
    // SKILL.md 经 renderMarkdown（净化管线）渲染成 HTML：标题进了 <h1>。
    const h1 = preview.querySelector('.skills-md h1')
    expect(h1?.textContent).toBe('beads')
    expect(preview.querySelector('.skills-md code')?.textContent).toContain('bd ready')
    // 再点收起。
    skillRows(el)
      .find((r) => r.dataset.name === 'beads')!
      .querySelector<HTMLElement>('.skills-row-name')!
      .click()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="skill-preview"]')).toBeNull()
    el.remove()
  })

  it('never offers create or edit: toolbar carries only Refresh and Sync', async () => {
    const el = await mount()
    await goto(el, "skills")
    const buttons = waButtons(el).map((b) => b.textContent?.trim())
    expect(buttons).toContain('刷新')
    expect(buttons).toContain('同步')
    expect(buttons.some((t) => t?.includes('新建') || t?.includes('编辑'))).toBe(false)
    // 行内也没有编辑入口（只有删除 🗑）。
    const rowActions = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.skills-row .row-action')]
    expect(rowActions.length).toBe(3)
    expect(rowActions.every((b) => b.title === '删除')).toBe(true)
    el.remove()
  })

  it('delete confirm dialog states backend copies are cleaned at next Sync; confirming deletes and refreshes', async () => {
    const el = await mount()
    await goto(el, "skills")
    const callsBefore = apiMocks.skillsList.mock.calls.length
    skillRows(el)
      .find((r) => r.dataset.name === 'broken')!
      .querySelector<HTMLElement>('button[title="删除"]')!
      .click()
    await el.updateComplete
    // 确认文案讲明两段式语义：backend 副本在下次 Sync 清理。
    const text = el.shadowRoot!.querySelector('[data-testid="skill-delete-text"]')!.textContent!
    expect(text).toContain('broken')
    expect(text).toContain('同步')
    // 取消不发请求。
    el.shadowRoot!
      .querySelector('wa-dialog.skill-delete')!
      .querySelector<HTMLElement>('wa-button[appearance="plain"]')!
      .click()
    await el.updateComplete
    expect(apiMocks.skillsDelete).not.toHaveBeenCalled()
    // 确认 → DELETE + 列表重取。
    skillRows(el)
      .find((r) => r.dataset.name === 'broken')!
      .querySelector<HTMLElement>('button[title="删除"]')!
      .click()
    await el.updateComplete
    el.shadowRoot!
      .querySelector('wa-dialog.skill-delete')!
      .querySelector<HTMLElement>('wa-button[variant="danger"]')!
      .click()
    await settle(el)
    expect(apiMocks.skillsDelete).toHaveBeenCalledWith('broken')
    expect(apiMocks.skillsList.mock.calls.length).toBeGreaterThan(callsBefore)
    // 组件 state 已归零（对话框关闭的真源断言，与 forceStop 同姿态）。
    expect((el as unknown as { skillDelete: unknown }).skillDelete).toBeNull()
    el.remove()
  })

  it('Sync renders a result panel with per-backend counts and the no-placement list', async () => {
    const el = await mount()
    await goto(el, "skills")
    waButtons(el)
      .find((b) => b.textContent?.trim() === '同步')!
      .click()
    await settle(el)
    const panel = el.shadowRoot!.querySelector('[data-testid="skills-sync-result"]')!
    const text = panel.textContent ?? ''
    expect(text).toContain('claude')
    expect(text).toContain('已写入 1')
    expect(text).toContain('覆盖 1')
    expect(text).toContain('删除 1')
    expect(text).toContain('私有 2')
    // 无落点 backend 如实呈现（reported, not skipped）。
    expect(text).toContain('无落点：gemini')
    // 覆盖/删除的条目名逐一点名（「仓 wins」必须可见）。
    expect(panel.textContent).toContain('~ grill-me')
    expect(panel.textContent).toContain('- old-skill')
    el.remove()
  })

  it('renders an inline error instead of an empty list when /api/skills fails', async () => {
    apiMocks.skillsList.mockRejectedValue(new ApiError(500, 'skills listing exploded'))
    const el = await mount()
    await goto(el, "env-vars")
    await goto(el, "skills")
    const err = el.shadowRoot!.querySelector('.callout-error[role="alert"]')
    expect(err).toBeTruthy()
    expect(err!.textContent).toContain('skills listing exploded')
    expect(el.shadowRoot!.querySelector('[data-testid="skill-row"]')).toBeNull()
    el.remove()
  })

  it('Refresh re-reads the store: on-disk additions appear, kept previews survive, vanished previews collapse', async () => {
    const el = await mount()
    await goto(el, "skills")
    // 打开 beads 预览（spec：refresh 后预览属于当前视图状态）。
    skillRows(el)
      .find((r) => r.dataset.name === 'beads')!
      .querySelector<HTMLElement>('.skills-row-name')!
      .click()
    await settle(el)
    expect(el.shadowRoot!.querySelector('[data-testid="skill-preview"]')).toBeTruthy()

    // 模拟社区工具在盘上动了仓：新增 git-cloned、broken 被删。
    apiMocks.skillsList.mockResolvedValue({
      skills: [
        { name: 'beads', description: 'beads 工作流', attachments: [], valid: true },
        { name: 'git-cloned', description: '来自 git clone', attachments: [], valid: true },
      ],
    })
    waButtons(el)
      .find((b) => b.textContent?.trim() === '刷新')!
      .click()
    await settle(el)

    const rows = skillRows(el)
    // 新落盘条目出现；已消失条目退场。
    expect(rows.some((r) => r.dataset.name === 'git-cloned')).toBe(true)
    expect(rows.some((r) => r.dataset.name === 'broken')).toBe(false)
    expect(el.shadowRoot!.querySelector('.provider-toolbar span.label')?.textContent).toContain(
      '仓内 2 个技能',
    )
    // 预览目标仍在 → 预览保留。
    expect(el.shadowRoot!.querySelector('[data-testid="skill-preview"]')).toBeTruthy()

    // 预览目标随刷新消失 → 预览收起（详情随条目一起没了）。
    apiMocks.skillsList.mockResolvedValue({
      skills: [{ name: 'git-cloned', description: '来自 git clone', attachments: [], valid: true }],
    })
    waButtons(el)
      .find((b) => b.textContent?.trim() === '刷新')!
      .click()
    await settle(el)
    expect(el.shadowRoot!.querySelector('[data-testid="skill-preview"]')).toBeNull()
    el.remove()
  })

  it('renders the empty-store placeholder instead of a list when the store has no entries', async () => {
    apiMocks.skillsList.mockResolvedValue({ skills: [] })
    const el = await mount()
    await goto(el, "skills")
    expect(el.shadowRoot!.querySelector('.provider-toolbar span.label')?.textContent).toContain(
      '仓内 0 个技能',
    )
    const placeholder = el.shadowRoot!.querySelector('.prefs-placeholder')
    expect(placeholder).toBeTruthy()
    expect(placeholder!.textContent).toContain('技能仓为空')
    expect(placeholder!.textContent).toContain('sebas skills add')
    expect(skillRows(el)).toHaveLength(0)
    el.remove()
  })

  it('sync failure renders the inline error callout instead of a result panel and unsets busy', async () => {
    const el = await mount()
    await goto(el, "skills")
    apiMocks.skillsSync.mockRejectedValue(new ApiError(500, 'reconcile exploded'))
    waButtons(el)
      .find((b) => b.textContent?.trim() === '同步')!
      .click()
    await settle(el)

    const err = el.shadowRoot!.querySelector('[data-testid="skills-sync-error"]')
    expect(err).toBeTruthy()
    expect(err!.textContent).toContain('同步失败')
    expect(err!.textContent).toContain('reconcile exploded')
    expect(el.shadowRoot!.querySelector('[data-testid="skills-sync-result"]')).toBeNull()
    // busy 复位：按钮恢复可点（否则一次失败就永久卡死 Sync）。
    const syncBtn = waButtons(el).find((b) => b.textContent?.trim() === '同步')!
    expect((syncBtn as unknown as HTMLButtonElement).disabled).toBe(false)
    el.remove()
  })

  it('preview surfaces a detail fetch failure inline instead of a skeleton', async () => {
    const el = await mount()
    await goto(el, "skills")
    apiMocks.skillDetail.mockRejectedValue(new ApiError(404, '仓里没有条目 "beads"'))
    skillRows(el)
      .find((r) => r.dataset.name === 'beads')!
      .querySelector<HTMLElement>('.skills-row-name')!
      .click()
    await settle(el)

    // 详情失败在行内出 callout（错误分支直接挂在行上，无 preview 容器），
    // 且不冒充正文：无 markdown 渲染区。
    const row = skillRows(el).find((r) => r.dataset.name === 'beads')!
    expect(row.querySelector('.callout-error[role="alert"]')).toBeTruthy()
    expect(row.textContent).toContain('仓里没有条目')
    expect(row.querySelector('.skills-md')).toBeNull()
    el.remove()
  })

  it('invalid entry preview states the missing SKILL.md honestly (text null on the wire)', async () => {
    const el = await mount()
    await goto(el, "skills")
    apiMocks.skillDetail.mockResolvedValue({ name: 'broken', text: null, attachments: [] })
    skillRows(el)
      .find((r) => r.dataset.name === 'broken')!
      .querySelector<HTMLElement>('.skills-row-name')!
      .click()
    await settle(el)

    const preview = el.shadowRoot!.querySelector('[data-testid="skill-preview"]')!
    expect(preview.textContent).toContain('SKILL.md 缺失')
    expect(preview.querySelector('.skills-md')).toBeNull()
    el.remove()
  })
})

/**
 * Agents 分区（add-agent-settings-and-session-titles 5.1）。导航序（默认
 * role=null 挂载）：generic(0) appearance(1) services(2) models(3)
 * agents(4) skills(5) env-vars(6) about(7)。本 describe 全用 index 4。
 */
describe('add-agent-settings-and-session-titles：Agents 分区（/api/agents*）', () => {
  /** 导航序（默认 role=null 挂载）：generic(0) appearance(1) services(2)
   * models(3) agents(4) skills(5) env-vars(6) about(7)。 */
  function agentRows(el: SebasSettingsModal): HTMLElement[] {
    return [...el.shadowRoot!.querySelectorAll<HTMLElement>('[data-testid="agent-row"]')]
  }

  /** 按 label 找对话框（同一弹窗面里 provider 编辑器等对话框常驻 DOM，
   * 必须 scoped 取按钮，避免误点同文案的别家按钮）。 */
  function dialogByLabel(el: SebasSettingsModal, label: string): HTMLDialogElement {
    const dlg = [
      ...el.shadowRoot!.querySelectorAll<HTMLDialogElement>('wa-dialog'),
    ].find((d) => d.getAttribute('label') === label)
    expect(dlg, `dialog ${label} must exist`).toBeTruthy()
    return dlg!
  }

  function waButtonsIn(root: Element): HTMLElement[] {
    return [...root.querySelectorAll<HTMLElement>('wa-button')]
  }

  function setWaInput(el: SebasSettingsModal, label: string, value: string): void {
    const input = [
      ...el.shadowRoot!.querySelectorAll<HTMLInputElement>('wa-input'),
    ].find((i) => i.getAttribute('label') === label)
    expect(input, `wa-input ${label} must exist`).toBeTruthy()
    input!.value = value
    input!.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  }

  it('lazily loads the catalog on first visit; native is read-only with a builtIn badge and no actions', async () => {
    const el = await mount()
    await settle(el)
    expect(apiMocks.agents).not.toHaveBeenCalled()
    await goto(el, "agents")
    expect(el.section).toBe('agents')
    expect(apiMocks.agents).toHaveBeenCalledTimes(1)
    const native = el.shadowRoot!.querySelector('[data-testid="agent-row-native"]')
    expect(native).toBeTruthy()
    expect(native!.querySelector('[data-testid="agent-builtin-badge"]')?.textContent).toContain(
      '内置',
    )
    // native 行不得有编辑/删除入口。
    expect(native!.querySelector('button[title="编辑"]')).toBeNull()
    expect(native!.querySelector('button[title="删除"]')).toBeNull()
    // store 行有编辑/删除入口。
    const row = agentRows(el).find((r) => r.dataset['id'] === 'claude')!
    expect(row.querySelector('button[title="编辑"]')).toBeTruthy()
    expect(row.querySelector('button[title="删除"]')).toBeTruthy()
    el.remove()
  })

  it('create form submits the claude-shape payload with the typed id', async () => {
    apiMocks.agentsCreate.mockResolvedValue({ created: 'myclaude' })
    const el = await mount()
    await goto(el, "agents")
    waButtons(el)
      .find((b) => b.textContent?.includes('新建 agent'))!
      .click()
    await settle(el)
    const dlg = dialogByLabel(el, '新建 agent')
    setWaInput(el, 'Agent id', 'myclaude')
    // 形态下拉缺省 claude、路径预填 claude——直接保存。
    const save = waButtonsIn(dlg).find((b) => b.textContent?.trim() === '保存')!
    save.click()
    await settle(el)
    // （fix-webui-qa-round2 2.4）spawn 目录/参数字段随 create 全量上 wire
    // （留空 = null = 缺省/清除）。
    expect(apiMocks.agentsCreate).toHaveBeenCalledWith('myclaude', {
      driver: 'claude',
      path: 'claude',
      display: null,
      args: null,
      work_dir: null,
      sessions_dir: null,
    })
    el.remove()
  })

  // （fix-webui-qa-round6 4.1）保存读取**渲染中字段**的实况值：即使 @input
  // 事件链在任何一环丢失（Web Awesome 转发层——QA GUI 实测宿主与内层 input
  // 的 value 都已同步、保存仍报「agent id 必填」），保存仍以 DOM 为准。
  it('save reads the live DOM value even when no input event fired (round6 4.1)', async () => {
    apiMocks.agentsCreate.mockResolvedValue({ created: 'liveclaud' })
    const el = await mount()
    await goto(el, "agents")
    waButtons(el)
      .find((b) => b.textContent?.includes('新建 agent'))!
      .click()
    await settle(el)
    const dlg = dialogByLabel(el, '新建 agent')
    // 直接设宿主 value，**不派发** input 事件——组件状态保持空串。
    const host = dlg.querySelector<HTMLInputElement>('wa-input[data-testid="agent-form-id"]')!
    expect(host).toBeTruthy()
    host.value = 'liveclaud'
    await el.updateComplete
    const save = waButtonsIn(dlg).find((b) => b.textContent?.trim() === '保存')!
    save.click()
    await settle(el)
    expect(apiMocks.agentsCreate).toHaveBeenCalledTimes(1)
    expect(apiMocks.agentsCreate).toHaveBeenCalledWith('liveclaud', {
      driver: 'claude',
      path: 'claude',
      display: null,
      args: null,
      work_dir: null,
      sessions_dir: null,
    })
    el.remove()
  })

  // （fix-webui-qa-round6 4.2）关闭（取消）后同一设置会话内再点「＋新建
  // agent」：全新空表单打开、字段可写——重开不再被 hide 动画竞态吞掉。
  it('the create form reopens fresh after close in the same settings session (round6 4.2)', async () => {
    const el = await mount()
    await goto(el, "agents")
    const openForm = async () => {
      waButtons(el)
        .find((b) => b.textContent?.includes('新建 agent'))!
        .click()
      await settle(el)
    }
    await openForm()
    const first = dialogByLabel(el, '新建 agent')
    const firstId = first.querySelector<HTMLInputElement>('wa-input[data-testid="agent-form-id"]')!
    expect(firstId).toBeTruthy()
    firstId.value = 'something'
    await el.updateComplete
    // 取消关闭：agentForm=null → 对话框整棵移出 DOM（条件渲染）。
    waButtonsIn(first)
      .find((b) => b.textContent?.trim() === '取消')!
      .click()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('wa-dialog[label="新建 agent"]')).toBeNull()

    // 再开：全新空表单，id 字段渲染且为空（不残留上次输入）。
    await openForm()
    const second = dialogByLabel(el, '新建 agent')
    const secondId = second.querySelector<HTMLInputElement>('wa-input[data-testid="agent-form-id"]')!
    expect(secondId).toBeTruthy()
    expect(secondId.value).toBe('')
    // 字段可写（设值后保存带值提交）。
    secondId.value = 'reopened'
    await el.updateComplete
    apiMocks.agentsCreate.mockResolvedValue({ created: 'reopened' })
    waButtonsIn(second)
      .find((b) => b.textContent?.trim() === '保存')!
      .click()
    await settle(el)
    expect(apiMocks.agentsCreate).toHaveBeenCalledWith('reopened', expect.anything())
    el.remove()
  })

  it('create form rejects the reserved native id without a request', async () => {
    const el = await mount()
    await goto(el, "agents")
    waButtons(el)
      .find((b) => b.textContent?.includes('新建 agent'))!
      .click()
    await settle(el)
    setWaInput(el, 'Agent id', 'native')
    const dlg = dialogByLabel(el, '新建 agent')
    waButtonsIn(dlg)
      .find((b) => b.textContent?.trim() === '保存')!
      .click()
    await settle(el)
    expect(apiMocks.agentsCreate).not.toHaveBeenCalled()
    expect(
      el.shadowRoot!.querySelector('[data-testid="agent-form-error"]')!.textContent,
    ).toContain('native')
    el.remove()
  })

  it('edit defaults to keeping the launch definition and submits only the display change', async () => {
    apiMocks.agentsUpdate.mockResolvedValue({ updated: 'claude' })
    const el = await mount()
    await goto(el, "agents")
    agentRows(el)
      .find((r) => r.dataset['id'] === 'claude')!
      .querySelector<HTMLElement>('button[title="编辑"]')!
      .click()
    await settle(el)
    setWaInput(el, '显示名（可选）', 'My Claude')
    const dlg = dialogByLabel(el, '编辑 agent claude')
    waButtonsIn(dlg)
      .find((b) => b.textContent?.trim() === '保存')!
      .click()
    await settle(el)
    // 部分更新：display + spawn 目录/参数改动面（launch 形态保留存量——
    // 合并语义逐字段生效；fix-webui-qa-round2 2.4）。
    expect(apiMocks.agentsUpdate).toHaveBeenCalledWith('claude', {
      display: 'My Claude',
      work_dir: null,
      args: null,
    })
    el.remove()
  })

  it('delete is a confirmed action and calls the DELETE endpoint on confirm', async () => {
    apiMocks.agentsDelete.mockResolvedValue({ deleted: 'claude' })
    const el = await mount()
    await goto(el, "agents")
    agentRows(el)
      .find((r) => r.dataset['id'] === 'claude')!
      .querySelector<HTMLElement>('button[title="删除"]')!
      .click()
    await settle(el)
    // 确认弹窗打开、请求未发。
    expect(apiMocks.agentsDelete).not.toHaveBeenCalled()
    const dlg = dialogByLabel(el, '删除 agent')
    waButtonsIn(dlg)
      .find((b) => b.textContent?.trim() === '删除')!
      .click()
    await settle(el)
    expect(apiMocks.agentsDelete).toHaveBeenCalledWith('claude')
    // 删除后目录刷新 + 成功提示。
    expect(el.shadowRoot!.querySelector('[data-testid="agent-action"]')?.textContent).toContain(
      '已删除',
    )
    el.remove()
  })

  // ── fix-webui-qa-round2 2.4（M-A4+D-A4）：表单覆盖 spawn 关键字段 + display 回填 ──

  it('edit prefills display from the raw value and maps sessions/work/args into the payload', async () => {
    apiMocks.agents.mockResolvedValue({
      agents: [
        {
          id: 'myagent',
          display: 'myagent',
          display_raw: 'My Agent!',
          reachable: true,
          driver_raw: 'claude',
          args: ['--scenario', 'thinking'],
          work_dir: 'D:/sb/work',
          sessions_dir: 'D:/sb/sessions',
        },
      ],
    })
    apiMocks.agentsUpdate.mockResolvedValue({ updated: 'myagent' })
    const el = await mount()
    await goto(el, "agents")
    agentRows(el)
      .find((r) => r.dataset['id'] === 'myagent')!
      .querySelector<HTMLElement>('button[title="编辑"]')!
      .click()
    await settle(el)
    const dlg = dialogByLabel(el, '编辑 agent myagent')
    // D-A4：display 以未兜底的 display_raw 回填（display === id 不再丢失）。
    const displayInput = [...dlg.querySelectorAll('wa-input')].find(
      (i) => i.getAttribute('label') === '显示名（可选）',
    )
    expect(
      (displayInput as unknown as { value: string } | null)?.value,
    ).toBe('My Agent!')
    // M-A4：存量 launch 字段逐项回填。
    const sessionsInput = [...dlg.querySelectorAll('wa-input')].find(
      (i) => i.getAttribute('label') === '会话目录（可选）',
    )
    expect(
      (sessionsInput as unknown as { value: string } | null)?.value,
    ).toBe('D:/sb/sessions')
    // 改 args 后保存：claude 驱动的 put 携带三个 spawn 字段。
    const argsInput = [...dlg.querySelectorAll('wa-input')].find(
      (i) => i.getAttribute('label') === '启动参数（空格分隔）',
    ) as unknown as HTMLInputElement
    argsInput.value = '--scenario drip'
    argsInput.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    waButtonsIn(dlg)
      .find((b) => b.textContent?.trim() === '保存')!
      .click()
    await settle(el)
    expect(apiMocks.agentsUpdate).toHaveBeenCalledWith('myagent', {
      display: 'My Agent!',
      work_dir: 'D:/sb/work',
      // review P3 修复后：不改形态臂对 claude 行也携带 sessions_dir
      // （表单显示且可编辑的字段必须随保存上 wire，不再静默丢弃）。
      sessions_dir: 'D:/sb/sessions',
      args: ['--scenario', 'drip'],
    })
    el.remove()
  })

  // ── fix-webui-qa-round10 1.4（A-DEF-03）：dup-id 提示与实际行为一致 ──
  // 原 round2 3.3 的「新建重名 = 覆盖预告」被证伪（服务端实际 409 拒绝，
  // 覆盖从未发生）：新建路径不再预告覆盖，只呈现拒绝 error；编辑路径保存
  // 确实更新启动定义，覆盖提示只在它为真的地方出现。

  it('create with an existing id promises nothing — no overwrite warning (round10 1.4)', async () => {
    const el = await mount()
    await goto(el, "agents")
    waButtons(el)
      .find((b) => b.textContent?.trim() === '＋ 新建 agent')!
      .click()
    await settle(el)
    setWaInput(el, 'Agent id', 'claude')
    await settle(el)
    // 新建重名：无覆盖预告（spec「no warning promising an overwrite is
    // shown alongside the rejection」）。
    expect(
      el.shadowRoot!.querySelector('[data-testid="agent-duplicate-warning"]'),
    ).toBeNull()
    el.remove()
  })

  it('create with an existing id surfaces only the rejection error (round10 1.4)', async () => {
    apiMocks.agentsCreate.mockRejectedValue(
      Object.assign(new Error("agent 'claude' 已存在"), { status: 409 }),
    )
    const el = await mount()
    await goto(el, "agents")
    waButtons(el)
      .find((b) => b.textContent?.trim() === '＋ 新建 agent')!
      .click()
    await settle(el)
    setWaInput(el, 'Agent id', 'claude')
    await settle(el)
    const dlg = dialogByLabel(el, '新建 agent')
    waButtonsIn(dlg)
      .find((b) => b.textContent?.trim() === '保存')!
      .click()
    await settle(el)
    // 恰好一种结果：可见的拒绝 error，命名重复 id；无覆盖 warning 同框。
    const err = el.shadowRoot!.querySelector('[data-testid="agent-form-error"]')
    expect(err).toBeTruthy()
    expect(err!.textContent).toContain('claude')
    expect(err!.textContent).toContain('已存在')
    expect(
      el.shadowRoot!.querySelector('[data-testid="agent-duplicate-warning"]'),
    ).toBeNull()
    el.remove()
  })

  it('edit mode shows the overwrite warning, where it is true (round10 1.4)', async () => {
    apiMocks.agentsUpdate.mockResolvedValue({ updated: 'claude' })
    const el = await mount()
    await goto(el, "agents")
    agentRows(el)
      .find((r) => r.dataset['id'] === 'claude')!
      .querySelector<HTMLElement>('button[title="编辑"]')!
      .click()
    await settle(el)
    const warn = el.shadowRoot!.querySelector('[data-testid="agent-duplicate-warning"]')
    expect(warn).toBeTruthy()
    expect(warn!.textContent).toContain('更新')
    expect(warn!.textContent).toContain('claude')
    el.remove()
  })
})

describe('gate-agent-directory-writes 2.1：Agents 写入口随角色裁剪', () => {
  /** 带角色挂载（role=null = 鉴权关闭的宿主，保持既有可用）。 */
  async function mountAs(
    role: 'root' | 'admin' | 'member' | 'viewer' | null,
  ): Promise<SebasSettingsModal> {
    const el = document.createElement('sebas-settings-modal') as SebasSettingsModal
    if (role !== null) el.role = role
    el.open = true
    document.body.appendChild(el)
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    return el
  }

  /** 切到 Agents 分区（导航按角色裁剪，序号不能写死）。 */
  async function gotoAgents(el: SebasSettingsModal): Promise<void> {
    await goto(el, 'agents')
  }

  function newAgentButton(el: SebasSettingsModal): HTMLElement | null {
    return waButtons(el).find((b) => b.textContent?.includes('新建 agent')) ?? null
  }

  it('member/viewer 只读：New/Edit/Delete 全部不呈现（分区本身保留可浏览）', async () => {
    for (const role of ['member', 'viewer'] as const) {
      const el = await mountAs(role)
      await gotoAgents(el)
      expect(el.section).toBe('agents')
      // 目录仍加载（读保持登录门，浏览不受影响）。
      expect(apiMocks.agents).toHaveBeenCalled()
      expect(el.shadowRoot!.querySelector('[data-testid="agent-row-native"]')).toBeTruthy()
      expect(el.shadowRoot!.querySelectorAll('[data-testid="agent-row"]').length).toBeGreaterThan(0)
      // 写入口全部不呈现。
      expect(newAgentButton(el), `${role} 不得见「新建 agent」`).toBeNull()
      for (const row of el.shadowRoot!.querySelectorAll<HTMLElement>('[data-testid="agent-row"]')) {
        expect(row.querySelector('button[title="编辑"]'), `${role} 不得见「编辑」`).toBeNull()
        expect(row.querySelector('button[title="删除"]'), `${role} 不得见「删除」`).toBeNull()
      }
      el.remove()
    }
  })

  it('root/admin（及鉴权关闭宿主）写入口全在：New + 行内 Edit/Delete', async () => {
    for (const role of ['root', 'admin', null] as const) {
      const el = await mountAs(role)
      await gotoAgents(el)
      expect(newAgentButton(el), `${role} 必须见「新建 agent」`).toBeTruthy()
      const row = el
        .shadowRoot!
        .querySelector<HTMLElement>('[data-testid="agent-row"][data-id="claude"]')
      expect(row, `claude 行 @ ${role}`).toBeTruthy()
      expect(row!.querySelector('button[title="编辑"]')).toBeTruthy()
      expect(row!.querySelector('button[title="删除"]')).toBeTruthy()
      el.remove()
    }
  })

  it('空目录的引导文案随角色收窄：只读档不再指向隐藏的 New agent 入口', async () => {
    apiMocks.agents.mockResolvedValue({ agents: [{ id: 'native', display: 'Native Kernel', reachable: true }] })
    const el = await mountAs('viewer')
    await gotoAgents(el)
    const empty = el.shadowRoot!.querySelector('[data-testid="agents-empty"]')
    expect(empty).toBeTruthy()
    expect(empty!.textContent).not.toContain('新建 agent')
    el.remove()
  })
})

// ── fix-webui-qa-round10 3.2（C-DEF-02）：provider 变更面写入口随角色裁剪 ──
// spec「Member cannot mutate a provider … the settings UI does not offer the
// mutation controls to that member」：settings.manage 档（root/admin）才呈现
// 新建/设默认/清默认/行内编辑删除；member/viewer 只读浏览（列表保留）。
describe('fix-webui-qa-round10 3.2：Provider/默认选择写入口随角色裁剪', () => {
  async function mountAs(
    role: 'root' | 'admin' | 'member' | 'viewer' | null,
  ): Promise<SebasSettingsModal> {
    const el = document.createElement('sebas-settings-modal') as SebasSettingsModal
    if (role !== null) el.role = role
    el.open = true
    document.body.appendChild(el)
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    return el
  }

  async function gotoModels(el: SebasSettingsModal): Promise<void> {
    await goto(el, 'models')
  }

  it('member/viewer 只读：无新建/设默认/编辑/删除控件，列表仍可浏览', async () => {
    for (const role of ['member', 'viewer'] as const) {
      const el = await mountAs(role)
      await gotoModels(el)
      // 读面保留：provider 列表照常呈现。
      const row = el.shadowRoot!.querySelector<HTMLElement>('.provider-row')
      expect(row, `${role} 仍能浏览 provider 列表`).toBeTruthy()
      expect(row!.textContent).toContain('alpha')
      // 写控件全部不呈现。
      const buttons = [...el.shadowRoot!.querySelectorAll('wa-button')].map((b) =>
        b.textContent?.trim(),
      )
      expect(buttons.join('|'), `${role} 不得见「新建」`).not.toContain('新建（预设）')
      expect(buttons.join('|'), `${role} 不得见「新建（自定义）」`).not.toContain('新建（自定义）')
      for (const b of el.shadowRoot!.querySelectorAll<HTMLElement>('button[title]')) {
        const t = b.getAttribute('title') ?? ''
        expect(t, `${role} 不得见行内写控件: ${t}`).not.toBe('设为新建会话的默认')
        expect(t, `${role} 不得见行内写控件: ${t}`).not.toBe('编辑')
        expect(t, `${role} 不得见行内写控件: ${t}`).not.toBe('删除')
        expect(t, `${role} 不得见行内写控件: ${t}`).not.toBe('清除新建会话的默认值')
      }
      el.remove()
    }
  })

  it('root/admin（及鉴权关闭宿主）写控件全在：新建 + ★/✎/🗑 + 清默认', async () => {
    for (const role of ['root', 'admin', null] as const) {
      const el = await mountAs(role)
      await gotoModels(el)
      const toolbar = el.shadowRoot!.querySelector('.provider-toolbar')!
      expect(toolbar.textContent, `${role} 必须见「新建（预设）」`).toContain('新建（预设）')
      expect(toolbar.textContent, `${role} 必须见「新建（自定义）」`).toContain('新建（自定义）')
      const row = el
        .shadowRoot!
        .querySelector<HTMLElement>('.provider-row')
      expect(row!.querySelector('button[title="设为新建会话的默认"]'), `${role} ★`).toBeTruthy()
      expect(row!.querySelector('button[title="编辑"]'), `${role} ✎`).toBeTruthy()
      expect(row!.querySelector('button[title="删除"]'), `${role} 🗑`).toBeTruthy()
      el.remove()
    }
  })

  it('清默认控件（✕）仅在持有写权限且已设默认时呈现', async () => {
    const el = await mountAs('member')
    await gotoModels(el)
    // 组件 state 直写（defaults 读面独立于本测试的裁剪面）。
    ;(el as unknown as { defaults: { provider: string; model: string | null } | null }).defaults = {
      provider: 'alpha',
      model: null,
    }
    await el.updateComplete
    expect(
      el.shadowRoot!.querySelector('button[title="清除新建会话的默认值"]'),
      'member 已设默认也不得见清默认控件',
    ).toBeNull()
    el.remove()

    const el2 = await mountAs('admin')
    await gotoModels(el2)
    ;(el2 as unknown as { defaults: { provider: string; model: string | null } | null }).defaults = {
      provider: 'alpha',
      model: null,
    }
    await el2.updateComplete
    expect(
      el2.shadowRoot!.querySelector('button[title="清除新建会话的默认值"]'),
      'admin 已设默认时清默认控件在',
    ).toBeTruthy()
    el2.remove()
  })
})

describe('unify-router-process-shape：router 停止被拒的强制出口（D4）', () => {
  /** 装好带 router 行的 Services 分区，并在 confirm 弹窗里点掉 Disable。 */
  async function confirmDisableRouter(el: SebasSettingsModal): Promise<void> {
    await goto(el, "services")
    const routerCard = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')].find(
      (c) => c.querySelector('.service-id')?.textContent === 'router',
    )!
    routerCard.querySelector<HTMLElement>('button[title="停用服务"]')!.click()
    await el.updateComplete
    const confirm = el.shadowRoot!.querySelector(
      'wa-dialog.service-action-confirm',
    ) as HTMLElement
    confirm.querySelector<HTMLElement>('wa-button[variant="danger"]')!.click()
    await settle(el)
  }

  function forceDialog(el: SebasSettingsModal): HTMLElement {
    return el.shadowRoot!.querySelector('wa-dialog.service-force-stop') as HTMLElement
  }

  /**
   * 对话框开合的真源断言：组件 state `forceStop`。wa-dialog 的 open 属性
   * 回落走异步 requestClose 动画链（jsdom 无动画完成事件，时序不保证），
   * 与本文件既有 `editor` 断言同款读组件 state。
   */
  function forceStopState(el: SebasSettingsModal): { name: string; count: number | null } | null {
    return (el as unknown as { forceStop: { name: string; count: number | null } | null })
      .forceStop
  }

  /** 拒绝载荷（wire 合同主形态的解析产物：ApiError 携带 code + count）。 */
  function rejection(count: number): Error {
    return new ApiError(400, 'router has active routed sessions', 'active_routed_sessions', count)
  }

  beforeEach(() => {
    apiMocks.adminServicesSafe.mockResolvedValue({
      adapter_ok: true,
      services: [{ name: 'router', status: 'running', desired: 'running', uptime_secs: 30 }],
    })
    apiMocks.adminEventsSafe.mockResolvedValue({ adapter_ok: true, events: [] })
  })

  it('直接成功：首次 disable 不带 force，成功后刷新列表、不弹强制出口', async () => {
    const el = await mount()
    await confirmDisableRouter(el)
    expect(apiMocks.disableService).toHaveBeenCalledTimes(1)
    expect(apiMocks.disableService).toHaveBeenCalledWith('router', false)
    expect(forceStopState(el)).toBeNull()
    // 成功 → loadServices 重取列表（初载 + 动作后 = 2 次）。
    expect(apiMocks.adminServicesSafe).toHaveBeenCalledTimes(2)
    el.remove()
  })

  it('拒绝后强制：二层对话框呈现计数与后果，Force stop 以 force 重发并刷新', async () => {
    apiMocks.disableService
      .mockRejectedValueOnce(rejection(2))
      .mockResolvedValueOnce({ operation_id: 'op-force', status: 'accepted', message: 'accepted' })
    const el = await mount()
    await confirmDisableRouter(el)
    // 二层对话框打开：拒绝驱动，携带目标与计数；拒绝不落内联错误。
    expect(forceStopState(el)).toEqual({ name: 'router', count: 2 })
    expect(forceDialog(el).textContent).toContain('2')
    expect(forceDialog(el).textContent).toContain('流式')
    expect(el.shadowRoot!.querySelector('.callout-error')).toBeNull()
    // 「强制停止」= 同一停止请求带 force: true 重发；成功后刷新列表。
    forceDialog(el).querySelector<HTMLElement>('wa-button[variant="danger"]')!.click()
    await settle(el)
    expect(apiMocks.disableService).toHaveBeenCalledTimes(2)
    expect(apiMocks.disableService).toHaveBeenLastCalledWith('router', true)
    expect(forceStopState(el)).toBeNull()
    expect(apiMocks.adminServicesSafe).toHaveBeenCalledTimes(2)
    el.remove()
  })

  it('拒绝后取消：不发任何请求，router 行保持原状', async () => {
    apiMocks.disableService.mockRejectedValueOnce(rejection(5))
    const el = await mount()
    await confirmDisableRouter(el)
    expect(forceStopState(el)).toEqual({ name: 'router', count: 5 })
    forceDialog(el).querySelector<HTMLElement>('wa-button[appearance="plain"]')!.click()
    await settle(el)
    // 取消 = 无第二次请求、列表不刷新、对话框关闭且行保持原状。
    expect(forceStopState(el)).toBeNull()
    expect(apiMocks.disableService).toHaveBeenCalledTimes(1)
    expect(apiMocks.adminServicesSafe).toHaveBeenCalledTimes(1)
    expect(el.shadowRoot!.textContent ?? '').toContain('状态 running')
    el.remove()
  })

  it('400 但无 active_routed_sessions 的失败仍走既有内联错误，不弹强制出口', async () => {
    apiMocks.disableService.mockRejectedValueOnce(new ApiError(400, 'watchdog rejected it'))
    const el = await mount()
    await confirmDisableRouter(el)
    expect(forceStopState(el)).toBeNull()
    expect(el.shadowRoot!.querySelector('.callout-error')).toBeTruthy()
    expect(el.shadowRoot!.textContent ?? '').toContain('watchdog rejected it')
    expect(apiMocks.disableService).toHaveBeenCalledTimes(1)
    el.remove()
  })
})

describe('sebas-settings-modal appearance section', () => {
  beforeEach(() => {
    // jsdom 对 matchMedia 的支持不保证存在/一致；显式钉死为深色 OS，
    // 让 "System 解析为 dark" 成为确定性断言。
    vi.stubGlobal(
      'matchMedia',
      vi.fn().mockReturnValue({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
    )
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  function themeOptions(el: SebasSettingsModal): HTMLElement[] {
    return [...el.shadowRoot!.querySelectorAll<HTMLElement>('.theme-option')]
  }

  it('offers System/Dark/Light with System pressed by default (stubbed OS → dark)', async () => {
    // 页面启动时由 main.ts 应用一次主题 class；测试环境里手动补上。
    applyThemeMode()
    const el = await mount()
    await goto(el, "appearance")
    const options = themeOptions(el)
    expect(options.map((b) => b.querySelector('.theme-option-label')?.textContent)).toEqual([
      '跟随系统',
      '深色',
      '浅色',
    ])
    expect(options[0]!.getAttribute('aria-pressed')).toBe('true')
    expect(options[1]!.getAttribute('aria-pressed')).toBe('false')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    el.remove()
  })

  it('choosing Light unsets wa-dark and persists sebas:theme=light', async () => {
    const el = await mount()
    await goto(el, "appearance")
    themeOptions(el)[2]!.click()
    await el.updateComplete
    expect(localStorage.getItem('sebas:theme')).toBe('light')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(false)
    expect(themeOptions(el)[2]!.getAttribute('aria-pressed')).toBe('true')
    expect(el.shadowRoot!.textContent).toContain('立即生效，保存在当前浏览器。')
    el.remove()
  })

  it('choosing Dark sets wa-dark and persists; System returns to following the OS', async () => {
    const el = await mount()
    await goto(el, "appearance")
    themeOptions(el)[1]!.click()
    await el.updateComplete
    expect(localStorage.getItem('sebas:theme')).toBe('dark')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    // 回到 system：jsdom 无 matchMedia → 跟随解析为 dark。
    themeOptions(el)[0]!.click()
    await el.updateComplete
    expect(localStorage.getItem('sebas:theme')).toBe('system')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    expect(el.shadowRoot!.textContent).toContain('系统当前为深色')
    el.remove()
  })
})

describe('sebas-settings-modal closing', () => {
  it('the close button shuts it and bubbles the close event', async () => {
    const el = await mount()
    const closed = vi.fn()
    el.addEventListener('close', closed)
    el.shadowRoot!.querySelector<HTMLButtonElement>('.close')!.click()
    await el.updateComplete
    expect(el.open).toBe(false)
    expect(closed).toHaveBeenCalledTimes(1)
    expect(el.shadowRoot!.querySelector('.panel')).toBeNull()
    el.remove()
  })

  it('Escape closes it', async () => {
    const el = await mount()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await el.updateComplete
    expect(el.open).toBe(false)
    el.remove()
  })

  it('clicking the backdrop (not the panel) closes it', async () => {
    const el = await mount()
    const overlay = el.shadowRoot!.querySelector('.overlay') as HTMLElement
    // 真实遮罩点击 = 同一指针序列的 pointerdown + click 都落在遮罩上。
    overlay.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, composed: true }))
    overlay.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }))
    await el.updateComplete
    expect(el.open).toBe(false)
    el.remove()
  })

  // （fix-webui-qa-round6 4.4）按下不在遮罩上的 click（面板内子元素在
  // mousedown→click 之间被移除后的重定向）不关闭——「无操作自关」的候选
  // 机制在这里被掐断。
  it('a click whose pointerdown did not start on the backdrop does not close it (round6 4.4)', async () => {
    const el = await mount()
    const overlay = el.shadowRoot!.querySelector('.overlay') as HTMLElement
    // 只有 click，没有遮罩上的 pointerdown（重定向序列）。
    overlay.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }))
    await el.updateComplete
    expect(el.open).toBe(true)
    // 反向序列（pointerdown 在面板内、click 冒到遮罩）同样不关：面板内
    // pointerdown 把锚点置 false。
    const panel = el.shadowRoot!.querySelector('.panel') as HTMLElement
    panel.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, composed: true }))
    overlay.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }))
    await el.updateComplete
    expect(el.open).toBe(true)
    el.remove()
  })

  it('a closed modal renders nothing', async () => {
    const el = await mount(false)
    expect(el.shadowRoot!.querySelector('.panel')).toBeNull()
    el.remove()
  })
})

it('states honestly that no global default is set (agent-defaults retired)', async () => {
  // workbench-agent-wire-fix 3.3：/api/agent-defaults 端点退役——provider
  // 列表不再渲染全局 default 徽章（默认 agent 改为项目级记忆）。

  const el = await mount()
  await goto(el, "models")

  const status = el.shadowRoot?.querySelector('.provider-toolbar [role="status"]')
  expect(status?.textContent ?? '').toContain('未设置默认')
  expect(el.shadowRoot?.querySelector('.provider-badge.default')).toBeNull()
  el.remove()
})

describe('revamp-settings-nav-and-models-editor：编辑器内 fetch 整单替换', () => {
  /** Models 分区里指定 provider 的行。 */
  function rowFor(el: SebasSettingsModal, name: string): HTMLElement {
    const rows = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.provider-row')]
    const row = rows.find(
      (r) => r.querySelector('.provider-row-name')?.textContent?.trim() === name,
    )
    expect(row).toBeTruthy()
    return row!
  }

  /** 打开指定 provider 的编辑器（行内 ✎）。 */
  async function openEditorFor(el: SebasSettingsModal, name: string): Promise<HTMLElement> {
    await goto(el, "models")
    rowFor(el, name).querySelector<HTMLButtonElement>('button[title="编辑"]')!.click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
    expect(dialog).toBeTruthy()
    return dialog
  }

  /** 新建（preset/custom）编辑器。 */
  async function openCreateEditor(el: SebasSettingsModal, label: string): Promise<HTMLElement> {
    await goto(el, "models")
    waButtons(el)
      .find((b) => b.textContent?.includes(label))!
      .click()
    await el.updateComplete
    return el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
  }

  function editorFetchButton(dialog: HTMLElement): HTMLButtonElement | null {
    return dialog.querySelector<HTMLButtonElement>('button[data-testid="fetch-models"]')
  }

  function entryRows(dialog: HTMLElement): HTMLElement[] {
    return [...dialog.querySelectorAll<HTMLElement>('[data-testid="model-entry"]')]
  }

  function entryIds(dialog: HTMLElement): string[] {
    return entryRows(dialog).map(
      (r) => (r.querySelector('wa-input') as unknown as { value: string }).value,
    )
  }

  async function saveEditor(el: SebasSettingsModal): Promise<void> {
    ;(
      el.shadowRoot!.querySelector('wa-dialog.provider-editor wa-button[variant="brand"]') as HTMLElement
    ).click()
    await settle(el)
  }

  beforeEach(() => {
    apiMocks.fetchProviderModels.mockResolvedValue({ provider: 'beta', models: ['m-pro', 'm-flash'] })
    apiMocks.providerUpdate.mockResolvedValue({ updated: 'beta' })
  })

  // 4.1 验收：入口从 provider 行内挪进编辑器——行上无 🔍，编辑器（preset
  // 派生与 custom 皆然）有，且点击即调 core 的抓取 op。
  it('moves the fetch entry into the editor for preset and custom providers alike', async () => {
    const el = await mount()
    await goto(el, "models")
    expect(apiMocks.fetchProviderModels).not.toHaveBeenCalled()
    // 行内没有任何 fetch 按钮（含有可用 base URL 的 alpha/beta）。
    expect(el.shadowRoot!.querySelector('button[data-testid="fetch-models"]')).toBeNull()

    for (const name of ['beta', 'alpha']) {
      const dialog = await openEditorFor(el, name)
      const btn = editorFetchButton(dialog)
      expect(btn).toBeTruthy()
      btn!.click()
      await settle(el)
      expect(apiMocks.fetchProviderModels).toHaveBeenCalledWith(name)
      // 关掉再开下一个。
      ;(dialog.querySelector('wa-button[appearance="plain"]') as HTMLElement).click()
      await el.updateComplete
    }
    el.remove()
  })

  // 4.2/D3 验收：成功 → 整单替换编辑器草稿列表；抓取零写请求；保存才落库。
  it('replaces the editor draft wholesale; storage moves only through the normal save', async () => {
    const el = await mount()
    const dialog = await openEditorFor(el, 'beta')
    expect(entryRows(dialog).length).toBe(0)

    editorFetchButton(dialog)!.click()
    await settle(el)

    expect(apiMocks.fetchProviderModels).toHaveBeenCalledWith('beta')
    expect(apiMocks.providerUpdate).not.toHaveBeenCalled()
    expect(apiMocks.providerCreate).not.toHaveBeenCalled()
    expect(entryIds(dialog)).toEqual(['m-pro', 'm-flash'])

    await saveEditor(el)
    expect(apiMocks.providerUpdate).toHaveBeenCalledTimes(1)
    expect(apiMocks.providerUpdate).toHaveBeenCalledWith('beta', {
      protocol: 'auto',
      base_url_anthropic: undefined,
      base_url_openai_chat: 'https://b.example/v1',
      base_url_openai_responses: undefined,
      api_key_env: undefined,
      default_model: undefined,
      model_map: undefined,
      models: [
        { id: 'm-pro', tags: [] },
        { id: 'm-flash', tags: [] },
      ],
    })
    el.remove()
  })

  // 4.2 验收：同 id 保留人工 capability tags、按 id 去重（上游可能回重复）。
  it('keeps manual capability tags for surviving ids and dedupes by id', async () => {
    apiMocks.fetchProviderModels.mockResolvedValue({
      provider: 'alpha',
      models: ['m2', 'm3', 'm2'],
    })
    const el = await mount()
    const dialog = await openEditorFor(el, 'alpha')
    // alpha 存量目录：m1、m2(vision)。抓取后骨架 = m2、m3（重复 m2 折叠），
    // m2 的人工 vision 保留。
    expect(entryIds(dialog)).toEqual(['m1', 'm2'])

    editorFetchButton(dialog)!.click()
    await settle(el)

    expect(entryIds(dialog)).toEqual(['m2', 'm3'])
    const vision = entryRows(dialog)[0]!.querySelector(
      'input[data-testid="tag-vision"]',
    ) as HTMLInputElement
    expect(vision.checked).toBe(true)

    await saveEditor(el)
    expect(apiMocks.providerUpdate).toHaveBeenCalledWith('alpha', {
      protocol: 'auto',
      preset: 'deepseek',
      models: [
        { id: 'm2', tags: ['vision'] },
        { id: 'm3', tags: [] },
      ],
    })
    el.remove()
  })

  // 4.3 验收：取消编辑器 = 丢弃抓取结果，存储不变。
  it('cancelling the editor discards the fetch', async () => {
    const el = await mount()
    const dialog = await openEditorFor(el, 'beta')
    editorFetchButton(dialog)!.click()
    await settle(el)
    expect(entryIds(dialog)).toEqual(['m-pro', 'm-flash'])

    ;(dialog.querySelector('wa-button[appearance="plain"]') as HTMLElement).click()
    await el.updateComplete
    expect((el as unknown as { editor: unknown }).editor).toBeNull()
    expect(apiMocks.providerUpdate).not.toHaveBeenCalled()

    // 重开编辑器：草稿回到存量目录（空），不是抓取结果。
    const dialog2 = await openEditorFor(el, 'beta')
    expect(entryRows(dialog2).length).toBe(0)
    el.remove()
  })

  // 3.3 验收（继承）：失败如实呈现净化原因、绝不冒充空成功列表；草稿不动。
  it('keeps the draft and reports the sanitized reason on failure', async () => {
    apiMocks.fetchProviderModels.mockRejectedValue(
      new ApiError(502, "fetch_models: provider 'beta' 上游抓取失败: HTTP 401 Unauthorized"),
    )
    const el = await mount()
    const dialog = await openEditorFor(el, 'beta')
    // 先手工加一条，证明失败后草稿原样保留。
    ;(dialog.querySelector('button[data-testid="add-model-entry"]') as HTMLButtonElement).click()
    await el.updateComplete
    const manual = entryRows(dialog)[0]!.querySelector('wa-input') as unknown as {
      value: string
      dispatchEvent: (e: Event) => boolean
    }
    manual.value = 'manual-1'
    manual.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    await el.updateComplete

    editorFetchButton(dialog)!.click()
    await settle(el)

    const err = dialog.querySelector('.fetch-error[role="alert"]')
    expect(err).toBeTruthy()
    expect(err!.textContent).toContain('HTTP 401')
    expect(entryIds(dialog)).toEqual(['manual-1'])
    expect(apiMocks.providerUpdate).not.toHaveBeenCalled()
    el.remove()
  })

  // （fix-webui-qa-round3 2.2 / D5）长探测错误完整可读：错误文本独占一行、
  // 任意点可断——不再被表单右缘单行裁切（QA W3 D2 实锤形态）。样式合同：
  // 头行允许换行、错误文本 flex-basis 100% + anywhere 断行。
  it('wraps long probe errors on their own full-width line (D5)', async () => {
    const el = await mount()
    const css = [...el.shadowRoot!.querySelectorAll('style')]
      .map((s) => s.textContent ?? '')
      .join('\n')
    const headRule = css.match(/\.model-entries \.entries-head\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(headRule).toMatch(/flex-wrap:\s*wrap/)
    const errRule = css.match(/\.model-entries \.fetch-error\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(errRule).toMatch(/flex:\s*1\s+1\s+100%/, '错误文本独占整行')
    expect(errRule).toMatch(/min-width:\s*0/)
    expect(errRule).toMatch(/overflow-wrap:\s*anywhere/)
    expect(errRule).toMatch(/white-space:\s*normal/)
    el.remove()
  })

  // spec「no base URL means no fetch entry」：三槽位全空的 provider，编辑器
  // 不渲染 fetch 动作。
  it('renders no fetch action in the editor for a provider without a usable base URL', async () => {
    apiMocks.providers.mockResolvedValue({
      providers: [
        {
          name: 'urlless',
          preset: null,
          base_url_anthropic: null,
          base_url_openai_chat: null,
          base_url_openai_responses: null,
          api_key_env: null,
          api_key_configured: false,
          models: [],
        },
      ],
    })
    const el = await mount()
    const dialog = await openEditorFor(el, 'urlless')
    expect(editorFetchButton(dialog)).toBeNull()
    el.remove()
  })

  // D4 边界：新建模式没有已存储的 provider 可探测（probe op 按 name 寻址），
  // 即便 presetDef 有 code-table URL 也不渲染 fetch 动作。
  it('renders no fetch action in create editors', async () => {
    const el = await mount()
    for (const label of ['新建（预设）', '新建（自定义）']) {
      const dialog = await openCreateEditor(el, label)
      expect(editorFetchButton(dialog)).toBeNull()
      ;(dialog.querySelector('wa-button[appearance="plain"]') as HTMLElement).click()
      await el.updateComplete
    }
    el.remove()
  })

  // 4.3 验收：fetch 在飞时按钮 disabled。
  it('disables the fetch button while the request is in flight', async () => {
    let release!: (v: { provider: string; models: string[] }) => void
    apiMocks.fetchProviderModels.mockReturnValue(
      new Promise((resolve) => {
        release = resolve
      }),
    )
    const el = await mount()
    const dialog = await openEditorFor(el, 'beta')
    const btn = editorFetchButton(dialog)!
    btn.click()
    await el.updateComplete
    expect(btn.disabled).toBe(true)
    release({ provider: 'beta', models: ['m-pro'] })
    await settle(el)
    expect(btn.disabled).toBe(false)
    expect(entryIds(dialog)).toEqual(['m-pro'])
    el.remove()
  })
})

describe('redesign-provider-models-settings 2.1：wa-hide 来源守卫', () => {
  async function openEditor(el: SebasSettingsModal): Promise<HTMLElement> {
    await goto(el, "models")
    const newBtn = waButtons(el).find((b) => b.textContent?.includes('新建（预设）'))!
    expect(newBtn).toBeTruthy()
    newBtn.click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
    expect(dialog).toBeTruthy()
    return dialog
  }

  it('a wa-hide bubbling from the inner wa-select does NOT close the editor', async () => {
    const el = await mount()
    const dialog = await openEditor(el)
    const select = dialog.querySelector('wa-select[label="预设"]')
    expect(select).toBeTruthy()
    // 模拟 WA <wa-select> 收起列表框：从子控件派发 composed wa-hide。
    select!.dispatchEvent(new CustomEvent('wa-hide', { bubbles: true, composed: true }))
    await el.updateComplete
    expect((el as unknown as { editor: unknown }).editor).not.toBeNull()
    expect(dialog.hasAttribute('open')).toBe(true)
    el.remove()
  })

  it('a wa-hide from the dialog itself still closes it', async () => {
    const el = await mount()
    const dialog = await openEditor(el)
    dialog.dispatchEvent(new CustomEvent('wa-hide', { bubbles: true, composed: true }))
    await el.updateComplete
    expect((el as unknown as { editor: unknown }).editor).toBeNull()
    el.remove()
  })
})

describe('redesign-provider-models-settings 3.1：预制最小表单', () => {
  /** 打开预制编辑器并等它渲染。 */
  async function openPresetEditor(el: SebasSettingsModal): Promise<HTMLElement> {
    await goto(el, "models")
    waButtons(el)
      .find((b) => b.textContent?.includes('新建（预设）'))!
      .click()
    await el.updateComplete
    return el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
  }

  /** 设 WA 表单控件的值并派发 input 事件（组件 @input 读 target.value）。 */
  function setWaValue(host: Element, selector: string, value: string): void {
    const input = host.querySelector(selector) as unknown as HTMLInputElement
    input.value = value
    input.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  }

  it('preset create needs only preset + key + model entries; name defaults to the preset name', async () => {
    const el = await mount()
    const dialog = await openPresetEditor(el)
    apiMocks.providerCreate.mockResolvedValue({ created: 'deepseek' })

    // 最小路径里没有实例名输入（D5：默认取 preset 名；改名在 Advanced）。
    expect(dialog.querySelector('wa-input[label="名称"]')).toBeNull()

    // API key。
    setWaValue(dialog, 'wa-input[label="API key"]', 'sk-test')
    // 两个模型条目：第一个纯文本，第二个带 vision。
    const addBtn = dialog.querySelector('button[data-testid="add-model-entry"]') as HTMLButtonElement
    addBtn.click()
    addBtn.click()
    await el.updateComplete
    const rows = [...dialog.querySelectorAll('[data-testid="model-entry"]')]
    expect(rows.length).toBe(2)
    const idInputs = rows.map(
      (r) => r.querySelector('wa-input') as unknown as { value: string; dispatchEvent: (e: Event) => boolean },
    )
    idInputs[0].value = 'model-a'
    idInputs[0].dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    idInputs[1].value = 'model-b'
    idInputs[1].dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    const visionBox = rows[1].querySelector(
      'input[data-testid="tag-vision"]',
    ) as HTMLInputElement
    visionBox.click()
    await el.updateComplete

    ;(
      el.shadowRoot!.querySelector('wa-dialog.provider-editor wa-button[variant="brand"]') as HTMLElement
    ).click()
    await settle(el)

    expect(apiMocks.providerCreate).toHaveBeenCalledTimes(1)
    const payload = apiMocks.providerCreate.mock.calls[0][0] as Record<string, unknown>
    expect(payload.preset).toBe('deepseek')
    expect(payload.api_key).toBe('sk-test')
    // 实例名缺省取预设名（D5）。
    expect(payload.name).toBe('deepseek')
    // 条目与能力标记原样提交（text 隐含不写）。
    expect(payload.models).toEqual([
      { id: 'model-a', tags: [] },
      { id: 'model-b', tags: ['vision'] },
    ])
    // 3.1 验收：不含 base_url_* 与 api_key_env。
    expect(payload.base_url_anthropic).toBeUndefined()
    expect(payload.base_url_openai_chat).toBeUndefined()
    expect(payload.base_url_openai_responses).toBeUndefined()
    expect(payload.api_key_env).toBeUndefined()
    el.remove()
  })

  it('untouched preset entries stay unsubmitted (catalog keeps following the code table)', async () => {
    const el = await mount()
    const dialog = await openPresetEditor(el)
    apiMocks.providerCreate.mockResolvedValue({ created: 'deepseek' })
    setWaValue(dialog, 'wa-input[label="API key"]', 'sk-only')
    ;(
      el.shadowRoot!.querySelector('wa-dialog.provider-editor wa-button[variant="brand"]') as HTMLElement
    ).click()
    await settle(el)
    const payload = apiMocks.providerCreate.mock.calls[0][0] as Record<string, unknown>
    expect(payload.models).toBeUndefined()
    el.remove()
  })

  it('the entries block reads "Models" with a bare full-width ＋ and no hint copy', async () => {
    const el = await mount()
    const dialog = await openPresetEditor(el)
    // revamp…3.1：区块标签改「Models」、两句提示语删除。
    expect(dialog.querySelector('.model-entries .entries-label')?.textContent?.trim()).toBe('模型')
    expect(dialog.querySelector('.entries-hint')).toBeNull()
    // 「＋ Add model」改纯 ＋ 通栏按钮。
    const add = dialog.querySelector('button[data-testid="add-model-entry"]') as HTMLButtonElement
    expect(add).toBeTruthy()
    expect(add.textContent?.trim()).toBe('＋')
    expect(add.classList.contains('add-model')).toBe(true)
    el.remove()
  })
})

describe('redesign-provider-models-settings 3.2：定制最小表单 + Advanced 折叠', () => {
  async function openCustomEditor(el: SebasSettingsModal): Promise<HTMLElement> {
    await goto(el, "models")
    waButtons(el)
      .find((b) => b.textContent?.includes('新建（自定义）'))!
      .click()
    await el.updateComplete
    return el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
  }

  function setWaValue(host: Element, selector: string, value: string): void {
    const input = host.querySelector(selector) as unknown as HTMLInputElement
    input.value = value
    input.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  }

  async function save(el: SebasSettingsModal): Promise<void> {
    ;(
      el.shadowRoot!.querySelector('wa-dialog.provider-editor wa-button[variant="brand"]') as HTMLElement
    ).click()
    await settle(el)
  }

  it('minimal custom create: name + base url + protocol; advanced collapsed by default', async () => {
    const el = await mount()
    const dialog = await openCustomEditor(el)
    apiMocks.providerCreate.mockResolvedValue({ created: 'my-api' })

    const advanced = dialog.querySelector('details.advanced') as HTMLDetailsElement
    expect(advanced).toBeTruthy()
    // 默认折叠。
    expect(advanced.open).toBe(false)

    setWaValue(dialog, 'wa-input[label="名称"]', 'my-api')
    // 协议缺省 OpenAI-compatible → 单个 Base URL 落 openai_chat 槽（D4）。
    setWaValue(dialog, 'wa-input[label="Base URL（OpenAI 兼容）"]', 'https://api.example/v1')
    setWaValue(dialog, 'wa-input[label="API key"]', 'sk-custom')

    await save(el)
    expect(apiMocks.providerCreate).toHaveBeenCalledTimes(1)
    const payload = apiMocks.providerCreate.mock.calls[0][0] as Record<string, unknown>
    expect(payload.name).toBe('my-api')
    expect(payload.protocol).toBe('openai')
    expect(payload.base_url_openai_chat).toBe('https://api.example/v1')
    // 默认提交不带 Advanced 独占字段：其余槽位 / 改名映射 / api_key_env。
    expect(payload.base_url_anthropic).toBeUndefined()
    expect(payload.base_url_openai_responses).toBeUndefined()
    expect(payload.model_map).toBeUndefined()
    expect(payload.api_key_env).toBeUndefined()
    el.remove()
  })

  it('expanding Advanced exposes the remaining slots and rename map for editing', async () => {
    const el = await mount()
    const dialog = await openCustomEditor(el)
    apiMocks.providerCreate.mockResolvedValue({ created: 'my-api' })

    const advanced = dialog.querySelector('details.advanced') as HTMLDetailsElement
    advanced.open = true
    await el.updateComplete

    setWaValue(dialog, 'wa-input[label="名称"]', 'my-api')
    setWaValue(dialog, 'wa-input[label="Base URL（OpenAI 兼容）"]', 'https://api.example/v1')
    // 展开后可编辑其余槽位（anthropic 槽 + responses 槽）与改名映射。
    setWaValue(dialog, 'wa-input[label="Base URL（Anthropic）"]', 'https://api.example/anthropic')
    setWaValue(
      dialog,
      'wa-input[label="Base URL（OpenAI Responses）"]',
      'https://api.example/responses',
    )
    setWaValue(
      dialog,
      'wa-input[label^="模型重命名映射"]',
      'old-model -> new-model',
    )
    await save(el)

    const payload = apiMocks.providerCreate.mock.calls[0][0] as Record<string, unknown>
    expect(payload.base_url_anthropic).toBe('https://api.example/anthropic')
    expect(payload.base_url_openai_responses).toBe('https://api.example/responses')
    expect(payload.model_map).toEqual({ 'old-model': 'new-model' })
    el.remove()
  })
})

describe('add-webui-multiuser-rbac 5.3/5.4：Users 分区与角色裁剪', () => {
  const usersFixture = [
    { id: 1, username: 'root', role: 'root', enabled: true, created_at_unix: 1_700_000_000 },
    { id: 2, username: 'alice', role: 'member', enabled: false, created_at_unix: 1_700_000_500 },
  ]

  /** 带角色挂载（role=null = 鉴权关闭的宿主，保持既有分区）。 */
  async function mountAs(
    role: 'root' | 'admin' | 'member' | 'viewer' | null,
  ): Promise<SebasSettingsModal> {
    const el = document.createElement('sebas-settings-modal') as SebasSettingsModal
    if (role !== null) el.role = role
    el.open = true
    document.body.appendChild(el)
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    return el
  }

  function navLabels(el: SebasSettingsModal): (string | undefined)[] {
    return navItems(el).map((b) => b.textContent?.trim())
  }

  async function gotoUsers(el: SebasSettingsModal): Promise<void> {
    await goto(el, 'users')
  }

  function userRow(el: SebasSettingsModal, username: string): HTMLElement {
    const row = [...el.shadowRoot!.querySelectorAll<HTMLElement>('[data-testid="user-row"]')].find(
      (r) => r.dataset['username'] === username,
    )
    expect(row).toBeTruthy()
    return row!
  }

  /** 设 WA 控件值并派发事件（input/change，组件 handler 读 target.value）。 */
  function setWaValue(host: Element, selector: string, value: string, event = 'input'): void {
    const input = host.querySelector(selector) as unknown as HTMLInputElement
    input.value = value
    input.dispatchEvent(new Event(event, { bubbles: true, composed: true }))
  }

  /**
   * 选 wa-select 的值并派发 change。jsdom 不触发 slotchange，晚于 connect
   * 加入的 wa-option 让 select 留下空选项缓存、value getter 过滤为 null
   * （见 new-session-dialog.test.ts 同款注释）——先显式 nudge 重建索引。
   */
  function pickWaSelect(host: Element, selector: string, value: string): void {
    const sel = host.querySelector(selector) as unknown as HTMLElement & {
      processSlotChange?: () => void
      value: string
    }
    sel.processSlotChange?.()
    sel.value = value
    sel.dispatchEvent(new Event('change', { bubbles: true, composed: true }))
  }

  function draftState(el: SebasSettingsModal, key: 'userCreate' | 'userReset' | 'userDelete'): unknown {
    return (el as unknown as Record<string, unknown>)[key]
  }

  beforeEach(() => {
    apiMocks.usersList.mockResolvedValue({ users: usersFixture })
  })

  it('trims the nav by role: Users is root-only, Services hides for member/viewer', async () => {
    const rootEl = await mountAs('root')
    expect(navLabels(rootEl)).toEqual([
      '通用',
      '外观',
      '服务',
      '用户',
      '模型',
      // （fix-webui-qa-round8 6.1）别名分区紧跟模型。
      '别名',
      'Agent',
      '技能',
      '环境变量',
      '关于',
    ])
    rootEl.remove()

    const adminEl = await mountAs('admin')
    expect(navLabels(adminEl)).toEqual([
      '通用',
      '外观',
      '服务',
      '模型',
      '别名',
      'Agent',
      '技能',
      '环境变量',
      '关于',
    ])
    adminEl.remove()

    // member/viewer：无服务控制（services.control）、无用户管理（users.manage）。
    for (const role of ['member', 'viewer'] as const) {
      const el = await mountAs(role)
      const labels = navLabels(el)
      expect(labels).not.toContain('用户')
      expect(labels).not.toContain('服务')
      el.remove()
    }

    // role 缺省（服务端未启用鉴权的宿主）：既有分区全保留，Users 仍不可见
    // （users.manage 需要服务端身份）。
    const hostEl = await mountAs(null)
    expect(navLabels(hostEl)).toEqual([
      '通用',
      '外观',
      '服务',
      '模型',
      '别名',
      'Agent',
      '技能',
      '环境变量',
      '关于',
    ])
    hostEl.remove()
  })

  it('a remembered section that the role may not see falls back to generic', async () => {
    localStorage.setItem('lastSettingsSection', 'users')
    const el = await mountAs('admin')
    await settle(el)
    expect(el.section).toBe('generic')
    el.remove()
  })

  it('lists users lazily with role / enabled / created date; no request before the visit', async () => {
    const el = await mountAs('root')
    expect(apiMocks.usersList).not.toHaveBeenCalled()
    await gotoUsers(el)
    expect(el.section).toBe('users')
    expect(apiMocks.usersList).toHaveBeenCalledTimes(1)
    const rows = [...el.shadowRoot!.querySelectorAll<HTMLElement>('[data-testid="user-row"]')]
    expect(rows.length).toBe(2)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('alice')
    expect(text).toContain('member')
    expect(text).toContain('已禁用')
    // created_at_unix → ISO 日期（列表不携带任何哈希字段可展示）。
    expect(text).toContain('创建于 2023-11-14')
    expect(userRow(el, 'alice').querySelector('wa-select.user-role')).toBeTruthy()
    el.remove()
  })

  it('renders an inline error instead of an empty list when /api/users fails', async () => {
    apiMocks.usersList.mockRejectedValue(new ApiError(403, 'forbidden: users.manage only'))
    const el = await mountAs('root')
    await gotoUsers(el)
    expect(el.shadowRoot!.querySelector('.toolbar-error')?.textContent).toContain(
      'forbidden: users.manage only',
    )
    expect(el.shadowRoot!.querySelectorAll('[data-testid="user-row"]').length).toBe(0)
    el.remove()
  })

  it('creates a user with {username, password, role} from the dialog and refreshes the list', async () => {
    const el = await mountAs('root')
    await gotoUsers(el)
    waButtons(el)
      .find((b) => b.textContent?.includes('新建用户'))!
      .click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.user-create') as HTMLElement
    setWaValue(dialog, 'wa-input[label="用户名"]', 'bob')
    setWaValue(dialog, 'wa-input[label^="密码"]', 'long-enough')
    pickWaSelect(dialog, 'wa-select[label="角色"]', 'admin')
    ;(dialog.querySelector('wa-button[variant="brand"]') as HTMLElement).click()
    await settle(el)
    expect(apiMocks.usersCreate).toHaveBeenCalledTimes(1)
    expect(apiMocks.usersCreate).toHaveBeenCalledWith('bob', 'long-enough', 'admin')
    // 成功才关闭对话框并重取列表（初载 + 刷新 = 2 次）。
    expect(draftState(el, 'userCreate')).toBeNull()
    expect(apiMocks.usersList).toHaveBeenCalledTimes(2)
    el.remove()
  })

  it('rejects a weak password locally without any request', async () => {
    const el = await mountAs('root')
    await gotoUsers(el)
    waButtons(el)
      .find((b) => b.textContent?.includes('新建用户'))!
      .click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.user-create') as HTMLElement
    setWaValue(dialog, 'wa-input[label="用户名"]', 'bob')
    setWaValue(dialog, 'wa-input[label^="密码"]', 'short')
    ;(dialog.querySelector('wa-button[variant="brand"]') as HTMLElement).click()
    await settle(el)
    expect(apiMocks.usersCreate).not.toHaveBeenCalled()
    expect(dialog.querySelector('[data-testid="user-create-error"]')?.textContent).toContain(
      '密码至少需要 8 个字符',
    )
    el.remove()
  })

  it('shows the server 409 message in place and keeps the create dialog open', async () => {
    apiMocks.usersCreate.mockRejectedValue(new ApiError(409, '用户名已被占用'))
    const el = await mountAs('root')
    await gotoUsers(el)
    waButtons(el)
      .find((b) => b.textContent?.includes('新建用户'))!
      .click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.user-create') as HTMLElement
    setWaValue(dialog, 'wa-input[label="用户名"]', 'alice')
    setWaValue(dialog, 'wa-input[label^="密码"]', 'long-enough')
    ;(dialog.querySelector('wa-button[variant="brand"]') as HTMLElement).click()
    await settle(el)
    expect(apiMocks.usersCreate).toHaveBeenCalledTimes(1)
    expect(dialog.querySelector('[data-testid="user-create-error"]')?.textContent).toContain(
      '用户名已被占用',
    )
    expect(draftState(el, 'userCreate')).not.toBeNull()
    el.remove()
  })

  it('row actions hit the role / enabled endpoints and surface results in place', async () => {
    const el = await mountAs('root')
    await gotoUsers(el)
    pickWaSelect(userRow(el, 'alice'), 'wa-select.user-role', 'viewer')
    await settle(el)
    expect(apiMocks.usersSetRole).toHaveBeenCalledWith(2, 'viewer')
    // 改角色触发列表重取后 DOM 重建：重新寻行再点启停。
    ;(userRow(el, 'root').querySelector('button[title="停用用户"]') as HTMLElement).click()
    await settle(el)
    expect(apiMocks.usersSetEnabled).toHaveBeenCalledWith(1, false)
    const callout = el.shadowRoot!.querySelector('[data-testid="user-action"]')
    expect(callout?.getAttribute('role')).toBe('status')
    expect(callout?.textContent).toContain('已禁用')
    el.remove()
  })

  it('a last-root 409 on role change shows the server message in place and reverts via refetch', async () => {
    apiMocks.usersSetRole.mockRejectedValue(new ApiError(409, '不能降级最后一个启用的 root'))
    const el = await mountAs('root')
    await gotoUsers(el)
    pickWaSelect(userRow(el, 'root'), 'wa-select.user-role', 'member')
    await settle(el)
    const callout = el.shadowRoot!.querySelector('[data-testid="user-action"]')
    expect(callout?.getAttribute('role')).toBe('alert')
    expect(callout?.textContent).toContain('不能降级最后一个启用的 root')
    // 失败也重取列表：行内 select 的显示值以服务端读数回滚，不假装改成功。
    expect(apiMocks.usersList).toHaveBeenCalledTimes(2)
    el.remove()
  })

  it('reset password goes through its dialog and reports success in place', async () => {
    const el = await mountAs('root')
    await gotoUsers(el)
    ;(userRow(el, 'alice').querySelector('button[title="重置密码"]') as HTMLElement).click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.user-reset') as HTMLElement
    expect(dialog.textContent).toContain('alice')
    setWaValue(dialog, 'wa-input', 'new-password')
    ;(dialog.querySelector('wa-button[variant="brand"]') as HTMLElement).click()
    await settle(el)
    expect(apiMocks.usersSetPassword).toHaveBeenCalledWith(2, 'new-password')
    expect(draftState(el, 'userReset')).toBeNull()
    expect(el.shadowRoot!.querySelector('[data-testid="user-action"]')?.textContent).toContain(
      '已重置',
    )
    el.remove()
  })

  it('delete confirmation shows the last-root protection message in place on 409', async () => {
    apiMocks.usersDelete.mockRejectedValue(new ApiError(409, '不能删除最后一个启用的 root'))
    const el = await mountAs('root')
    await gotoUsers(el)
    ;(userRow(el, 'root').querySelector('button[title="删除用户"]') as HTMLElement).click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.user-delete') as HTMLElement
    ;(dialog.querySelector('wa-button[variant="danger"]') as HTMLElement).click()
    await settle(el)
    expect(apiMocks.usersDelete).toHaveBeenCalledWith(1)
    expect(dialog.querySelector('[data-testid="user-delete-error"]')?.textContent).toContain(
      '不能删除最后一个启用的 root',
    )
    // 失败保持打开，操作者可取消。
    expect(draftState(el, 'userDelete')).not.toBeNull()
    el.remove()
  })
})

describe('no horizontal overflow in the settings modal (polish-workbench-walkthrough-ux 5.4)', () => {
  function styleCssText(): string {
    const styles = SettingsModalImpl.styles
    return (Array.isArray(styles) ? styles : [styles])
      .map((s) => (s as unknown as { cssText: string }).cssText)
      .join('\n')
  }

  it('content area kills horizontal overflow and lets long words wrap', () => {
    const css = styleCssText()
    // 内容区横向溢出就地消化（关闭按钮曾被裁切），长词断行。
    expect(css).toContain('overflow-x: hidden')
    expect(css).toContain('overflow-wrap: anywhere')
    // 表单/内嵌块允许收缩，min-width 下限交给内容自身。
    expect(css).toContain('min-width: 0')
  })
})

// ── fix-webui-qa-defects-round3：About 空值兜底（4.3）+ 命令名不断行（4.4）──

describe('About / Services display defects (round3 4.3/4.4)', () => {
  it('About toolchain：探测失败时点名成因，不再显示裸「未知」（fix-webui-qa-findings M9）', async () => {
    apiMocks.about.mockResolvedValue({
      uptime: '3h 12m',
      version: '0.4.2',
      build_time: '2026-09-29 08:30',
      git_branch: 'main',
      git_hash: 'abc1234',
      rustc: { state: 'error', cause: 'rustc 不在 PATH' },
      rustc_required: '1.90',
      router_listen: '127.0.0.1:8787',
      provider_count: 2,
      default_agent_kind: 'claude',
    })
    const el = await mount()
    await goto(el, "about")
    const row = [...el.shadowRoot!.querySelectorAll('dl.about-build .kv')].find((kv) =>
      kv.textContent?.includes('Rust 工具链'),
    )
    expect(row).toBeTruthy()
    // 探测失败 = 「探测失败（成因）」，不是裸「未知」。
    expect(row!.querySelector('[data-testid="about-toolchain"]')!.textContent).toContain('探测失败')
    expect(row!.querySelector('[data-testid="about-toolchain"]')!.textContent).toContain('rustc 不在 PATH')
    el.remove()
  })

  it('About toolchain：探测成功显示版本（fix-webui-qa-findings M9）', async () => {
    const el = await mount()
    await goto(el, "about")
    const row = [...el.shadowRoot!.querySelectorAll('dl.about-build .kv')].find((kv) =>
      kv.textContent?.includes('Rust 工具链'),
    )
    expect(row!.querySelector('[data-testid="about-toolchain"]')!.textContent).toContain('rustc 1.88.0')
    el.remove()
  })

  it('About toolchain：未安装显示「未安装」而非「未知」（fix-webui-qa-findings M9）', async () => {
    apiMocks.about.mockResolvedValue({
      uptime: '3h 12m',
      version: '0.4.2',
      build_time: '2026-09-29 08:30',
      git_branch: 'main',
      git_hash: 'abc1234',
      rustc: { state: 'missing', cause: '未安装（找不到 rustc 可执行文件）' },
      rustc_required: '1.90',
      router_listen: '127.0.0.1:8787',
      provider_count: 2,
      default_agent_kind: 'claude',
    })
    const el = await mount()
    await goto(el, "about")
    const row = [...el.shadowRoot!.querySelectorAll('dl.about-build .kv')].find((kv) =>
      kv.textContent?.includes('Rust 工具链'),
    )
    expect(row!.querySelector('[data-testid="about-toolchain"]')!.textContent).toContain('未安装')
    el.remove()
  })

  it('the Services no-adapter banner keeps the sebas run command unbreakable (4.4)', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({ adapter_ok: false, services: [] })
    const el = await mount()
    await goto(el, "services")
    const cmd = el.shadowRoot!.querySelector('code.run-cmd')
    expect(cmd, 'inline command carries the no-wrap class').toBeTruthy()
    expect(cmd!.textContent).toBe('sebas run')
    el.remove()
  })

  it('the run-cmd no-wrap rule ships in the modal styles (4.4)', () => {
    const styles = SettingsModalImpl.styles
    const css = (Array.isArray(styles) ? styles : [styles])
      .map((s) => (s as unknown as { cssText: string }).cssText)
      .join('\n')
    // .run-cmd 规则存在且为整体不断行（白空间策略）。
    expect(css).toContain('.run-cmd')
    expect(css).toMatch(/\.run-cmd\s*\{[^}]*white-space:\s*nowrap/)
  })
})

// ── add-about-build-info：BUILD 段构建时间行 + Git 信息行 ──

describe('About BUILD section shows build time and git info (add-about-build-info)', () => {
  it('renders a build-time row (UTC labelled) and a standalone git row under the version chip', async () => {
    const el = await mount()
    await goto(el, "about")
    const buildRows = [
      ...el.shadowRoot!.querySelectorAll('dl.about-build .kv'),
    ]
    // 行序（design D4）：Version → Build time → Git → 现状行（Uptime…）。
    const labels = buildRows.map((kv) => kv.querySelector('dt')?.textContent?.trim())
    expect(labels[0]).toBe('版本')
    expect(labels[1]).toBe('构建时间')
    expect(labels[2]).toBe('Git')
    expect(labels).toContain('运行时长')
    expect(labels).toContain('Rust 工具链')
    expect(labels).toContain('Router 监听')
    expect(labels).toContain('Provider 数')

    // 构建时间行：值 + UTC 标注（标注放展示层）。
    const buildTime = buildRows[1]!.querySelector('[data-testid="about-build-time"]')
    expect(buildTime!.textContent).toContain('2026-09-29 08:30')
    expect(buildTime!.textContent).toContain('UTC')

    // Git 行独立一行，形态 `分支名@短hash`。
    const git = buildRows[2]!.querySelector('[data-testid="about-git"]')
    expect(git!.textContent).toBe('main@abc1234')
    el.remove()
  })

  it('renders unknown values verbatim instead of hiding the rows', async () => {
    apiMocks.about.mockResolvedValue({
      uptime: '3h 12m',
      version: '0.4.2',
      build_time: 'unknown',
      git_branch: 'unknown',
      git_hash: 'unknown',
      rustc: { state: 'ok', version: 'rustc 1.88.0 (hash)' },
      rustc_required: '1.90',
      router_listen: '127.0.0.1:8787',
      provider_count: 2,
      default_agent_kind: 'claude',
    })
    const el = await mount()
    await goto(el, "about")
    // spec「构建信息缺失时如实呈现 unknown」：行不隐藏、照实显示。
    expect(
      el.shadowRoot!.querySelector('[data-testid="about-build-time"]')!.textContent,
    ).toContain('unknown')
    expect(el.shadowRoot!.querySelector('[data-testid="about-git"]')!.textContent).toBe(
      'unknown@unknown',
    )
    el.remove()
  })

  // ── fix-webui-qa-round2 3.1（D-A7）：toolchain 行的「要求 ≥」界限如实呈现 ──
  // 配置了界限才渲染标签；未配置不得出现悬空界限（有标签无值的行）。

  it('About toolchain：配置了最低版本时同 show「要求 ≥」界限 (round2 3.1)', async () => {
    const el = await mount()
    await goto(el, "about")
    const row = [...el.shadowRoot!.querySelectorAll('dl.about-build .kv')].find((kv) =>
      kv.textContent?.includes('Rust 工具链'),
    )
    const bound = row!.querySelector('.toolchain-required')
    expect(bound).toBeTruthy()
    expect(bound!.textContent).toContain('要求 ≥ 1.90')
    el.remove()
  })

  it('About toolchain：未配置界限时不渲染悬空的「要求 ≥」标签 (round2 3.1)', async () => {
    apiMocks.about.mockResolvedValue({
      uptime: '3h 12m',
      version: '0.4.2',
      build_time: '2026-09-29 08:30',
      git_branch: 'main',
      git_hash: 'abc1234',
      rustc: { state: 'ok', version: 'rustc 1.88.0 (hash)' },
      rustc_required: '',
      router_listen: '127.0.0.1:8787',
      provider_count: 2,
      default_agent_kind: 'claude',
    })
    const el = await mount()
    await goto(el, "about")
    const row = [...el.shadowRoot!.querySelectorAll('dl.about-build .kv')].find((kv) =>
      kv.textContent?.includes('Rust 工具链'),
    )
    // 探测值照常呈现，界限标签缺席。
    expect(row!.querySelector('[data-testid="about-toolchain"]')!.textContent).toContain(
      'rustc 1.88.0',
    )
    expect(row!.querySelector('.toolchain-required')).toBeNull()
    el.remove()
  })

})

// （fix-webui-qa-round8 7.4）复制按钮的两级可见反馈：成功 ✓、失败 ✗——
// 此前失败路径静默（QA round8 实锤「点击后无任何反馈」）。
describe('about copy button feedback (fix-webui-qa-round8 7.4)', () => {
  async function aboutView(): Promise<{ el: ReturnType<typeof mount> extends Promise<infer T> ? T : never; btn: HTMLButtonElement }> {
    const el = await mount()
    await goto(el, "about")
    const btn = el.shadowRoot!.querySelector<HTMLButtonElement>(
      'button[title="复制工作区根目录"]',
    )!
    expect(btn).toBeTruthy()
    return { el, btn }
  }

  it('clipboard success flips the button to ✓ (visible feedback)', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    })
    const { el, btn } = await aboutView()
    expect(btn.textContent?.trim()).toBe('⧉')
    await btn.click()
    await el.updateComplete
    // 1.5s 自复位由 window.setTimeout 承接（真实计时器），这里只钉两级反馈
    // 的「成功 ✓」半边。
    expect(btn.textContent?.trim()).toBe('✓')
    el.remove()
  })

  it('clipboard rejection falls back to execCommand and shows ✗ when that also fails', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('denied'))
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    })
    // jsdom 没有 execCommand：先定义再钉返回值。
    let execResult = false
    const exec = vi.fn((cmd: string) => (cmd === 'copy' ? execResult : false))
    Object.defineProperty(document, 'execCommand', { value: exec, configurable: true })
    const { el, btn } = await aboutView()
    await btn.click()
    await el.updateComplete
    expect(btn.textContent?.trim()).toBe('✗')
    // 兜底通路确实试过一次。
    expect(exec).toHaveBeenCalledWith('copy')
    el.remove()
  })
})

// （fix-webui-qa-round8 7.5）技能预览只渲染正文：SKILL.md 的 frontmatter
// 头块剥离后才进渲染管线，元数据键不得出现在预览里。
describe('skill preview strips frontmatter (fix-webui-qa-round8 7.5)', () => {
  it('the preview body starts at the prose, without the --- metadata block', async () => {
    const el = await mount()
    await goto(el, "skills")
    // skillRows 是分区 describe 内的局部助手：这里直接按 data-name 定位行。
    const beads = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.skills-row')].find(
      (r) => r.dataset.name === 'beads',
    )!
    beads.querySelector<HTMLElement>('.skills-row-name')!.click()
    await settle(el)
    const preview = el.shadowRoot!.querySelector('[data-testid="skill-preview"]')!
    const md = preview.querySelector('.skills-md')!
    // 正文在场。
    expect(md.querySelector('h1')?.textContent).toBe('beads')
    // frontmatter 键值不进预览（既是 7.5 的合同也是展示噪音的清除）。
    expect(md.textContent).not.toContain('description: beads')
    expect(md.textContent).not.toContain('name: beads')
    el.remove()
  })
})
