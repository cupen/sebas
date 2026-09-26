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

/** Click a nav entry by index and wait for its lazy loads to settle. */
async function goto(el: SebasSettingsModal, index: number): Promise<void> {
  navItems(el)[index]!.click()
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
    rustc_version: '1.88',
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
  it('renders the left nav with exactly Generic/Appearance/Services/Models/Skills/Env Vars/About', async () => {
    const el = await mount()
    const labels = navItems(el).map((b) => b.textContent?.trim())
    expect(labels).toEqual([
      'Generic',
      'Appearance',
      'Services',
      'Models',
      'Agents',
      'Skills',
      'Env Vars',
      'About',
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
    expect(tail.previousElementSibling?.textContent?.trim()).toBe('Skills')
    expect(tail.nextElementSibling?.classList.contains('nav-item')).toBe(true)
    expect(tail.nextElementSibling?.textContent?.trim()).toBe('Env Vars')
    // 底部组内 Env Vars → About 之间不再有分隔线（同组并列）。
    expect(tail.nextElementSibling?.nextElementSibling?.textContent?.trim()).toBe('About')
    // 另一条在 Services 项之前（appearance|services 组间线）。
    const plain = seps.find((s) => !s.classList.contains('tail'))!
    expect(plain.nextElementSibling?.textContent?.trim()).toBe('Services')
    expect(plain.previousElementSibling?.textContent?.trim()).toBe('Appearance')
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
    expect(text).toContain('will be provided here later')
    el.remove()
  })

  it('renders no maintenance actions anywhere (restart-all and reset retired)', async () => {
    const el = await mount()
    for (const index of [0, 1, 2, 3, 4, 5, 6, 7]) {
      await goto(el, index)
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
    await goto(el, 3)
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
    await goto(el, 2)
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
    expect(text).toContain('desired running · status running · up 1h 2m')
    expect(text).toContain('Recent errors')
    expect(text).toContain('im worker boom')
    // 动作按钮随 actual status 互斥（status-driven-service-rows）：im running
    // 只显 ■，router stopped 只显 ▶。
    const enables = [...el.shadowRoot!.querySelectorAll('button[title="Enable service"]')]
    expect(enables.length).toBe(1)
    const disables = [...el.shadowRoot!.querySelectorAll('button[title="Disable service"]')]
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
    await goto(el, 2)
    const cards = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')]
    const coreCard = cards.find((c) => c.querySelector('.service-id')?.textContent === 'core')!
    expect(coreCard).toBeTruthy()
    // core 纯只读（status-driven-service-rows D3）：无 ▶ / ■ / ⟳ 任何按钮。
    expect(coreCard.querySelector('button[title="Enable service"]')).toBeNull()
    expect(coreCard.querySelector('button[title="Disable service"]')).toBeNull()
    expect(coreCard.querySelector('button[title="Restart service"]')).toBeNull()
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
    await goto(el, 2)
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
    await goto(el, 2)
    const ids = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-id')].map((s) =>
      s.textContent?.trim(),
    )
    expect(ids).toEqual(['router'])
    el.remove()
  })

  it('Services section shows the no-adapter banner without rows or actions', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({ adapter_ok: false, services: [] })
    const el = await mount()
    await goto(el, 2)
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
    await goto(el, 2)
    const text = el.shadowRoot!.textContent ?? ''
    expect(text).toContain('router')
    expect(text).toContain('desired running · status running · up 1m')
    el.remove()
  })

  it('Models section renders the provider list only — no gateway card, no /api/router, no row-level fetch', async () => {
    const el = await mount()
    apiMocks.router.mockClear()
    await goto(el, 3)
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
    expect(text).toContain('deepseek · code')
    expect(text).toContain('custom')
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
    await goto(el, 7)
    expect(el.section).toBe('about')
    expect(apiMocks.about).toHaveBeenCalled()
    expect(apiMocks.fsBrowseDirs).toHaveBeenCalled()
    const text = el.shadowRoot!.textContent ?? ''
    // INSTANCE 段：工作区根目录 + default agent kind（读 /api/about 真值）。
    expect(text).toContain('Instance')
    expect(text).toContain('Workspace root')
    expect(text).toContain('/tmp/test-work')
    expect(text).toContain('Default agent kind')
    expect(text).toContain('claude')
    // BUILD 段：/api/about 真实字段。
    expect(text).toContain('Build')
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
    const copy = el.shadowRoot!.querySelector('button[title="Copy workspace root"]')
    expect(copy).toBeTruthy()
    el.remove()
  })

  it('About INSTANCE no longer renders the default provider/model row or a Models jump link', async () => {
    const el = await mount()
    await goto(el, 7)
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
    await goto(el, 7)
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
    await goto(el, 2)
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
    await goto(el, 2)
    const card = el.shadowRoot!.querySelector<HTMLElement>('.service-card')!
    const enable = card.querySelector<HTMLButtonElement>('button[title="Enable service"]')!
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
    await goto(el, 2)
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
    await goto(el, 6)
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
    await goto(el, 6)
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
    await goto(el, 6)
    const rows = envRows(el)
    expect(rows.length).toBe(1)
    expect(rows[0]!.querySelector('.value')?.textContent?.trim()).toBe('无法确定')
    el.remove()
  })

  it('renders an inline error instead of an empty table when /api/env fails', async () => {
    apiMocks.env.mockRejectedValue(new ApiError(500, 'env listing exploded'))
    const el = await mount()
    await goto(el, 6)
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
    await goto(el, 5)
    expect(el.section).toBe('skills')
    expect(apiMocks.skillsList).toHaveBeenCalledTimes(1)
    const rows = skillRows(el)
    expect(rows.length).toBe(3)
    // 名字 + attachment 数。
    const deploy = rows.find((r) => r.dataset.name === 'my-deploy')!
    expect(deploy.querySelector('.skills-row-atts')?.textContent?.trim()).toBe('2 attachments')
    // invalid 徽标 + 成因（描述位显 reason）。
    const broken = rows.find((r) => r.dataset.name === 'broken')!
    expect(broken.querySelector('[data-testid="skill-invalid"]')).toBeTruthy()
    expect(broken.querySelector('.skills-row-desc')?.textContent).toContain('围栏')
    el.remove()
  })

  it('clicking a row lazily loads the detail and renders sanitized markdown plus attachment list', async () => {
    const el = await mount()
    await goto(el, 5)
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
    await goto(el, 5)
    const buttons = waButtons(el).map((b) => b.textContent?.trim())
    expect(buttons).toContain('Refresh')
    expect(buttons).toContain('Sync')
    expect(buttons.some((t) => t?.includes('New') || t?.includes('Edit'))).toBe(false)
    // 行内也没有编辑入口（只有删除 🗑）。
    const rowActions = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.skills-row .row-action')]
    expect(rowActions.length).toBe(3)
    expect(rowActions.every((b) => b.title === 'Delete')).toBe(true)
    el.remove()
  })

  it('delete confirm dialog states backend copies are cleaned at next Sync; confirming deletes and refreshes', async () => {
    const el = await mount()
    await goto(el, 5)
    const callsBefore = apiMocks.skillsList.mock.calls.length
    skillRows(el)
      .find((r) => r.dataset.name === 'broken')!
      .querySelector<HTMLElement>('button[title="Delete"]')!
      .click()
    await el.updateComplete
    // 确认文案讲明两段式语义：backend 副本在下次 Sync 清理。
    const text = el.shadowRoot!.querySelector('[data-testid="skill-delete-text"]')!.textContent!
    expect(text).toContain('broken')
    expect(text).toContain('Sync')
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
      .querySelector<HTMLElement>('button[title="Delete"]')!
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
    await goto(el, 5)
    waButtons(el)
      .find((b) => b.textContent?.trim() === 'Sync')!
      .click()
    await settle(el)
    const panel = el.shadowRoot!.querySelector('[data-testid="skills-sync-result"]')!
    const text = panel.textContent ?? ''
    expect(text).toContain('claude')
    expect(text).toContain('1 written')
    expect(text).toContain('1 overwritten')
    expect(text).toContain('1 deleted')
    expect(text).toContain('2 private')
    // 无落点 backend 如实呈现（reported, not skipped）。
    expect(text).toContain('no placement: gemini')
    // 覆盖/删除的条目名逐一点名（「仓 wins」必须可见）。
    expect(panel.textContent).toContain('~ grill-me')
    expect(panel.textContent).toContain('- old-skill')
    el.remove()
  })

  it('renders an inline error instead of an empty list when /api/skills fails', async () => {
    apiMocks.skillsList.mockRejectedValue(new ApiError(500, 'skills listing exploded'))
    const el = await mount()
    await goto(el, 6)
    await goto(el, 5)
    const err = el.shadowRoot!.querySelector('.callout-error[role="alert"]')
    expect(err).toBeTruthy()
    expect(err!.textContent).toContain('skills listing exploded')
    expect(el.shadowRoot!.querySelector('[data-testid="skill-row"]')).toBeNull()
    el.remove()
  })

  it('Refresh re-reads the store: on-disk additions appear, kept previews survive, vanished previews collapse', async () => {
    const el = await mount()
    await goto(el, 5)
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
      .find((b) => b.textContent?.trim() === 'Refresh')!
      .click()
    await settle(el)

    const rows = skillRows(el)
    // 新落盘条目出现；已消失条目退场。
    expect(rows.some((r) => r.dataset.name === 'git-cloned')).toBe(true)
    expect(rows.some((r) => r.dataset.name === 'broken')).toBe(false)
    expect(el.shadowRoot!.querySelector('.provider-toolbar span.label')?.textContent).toContain(
      '2 skills in store',
    )
    // 预览目标仍在 → 预览保留。
    expect(el.shadowRoot!.querySelector('[data-testid="skill-preview"]')).toBeTruthy()

    // 预览目标随刷新消失 → 预览收起（详情随条目一起没了）。
    apiMocks.skillsList.mockResolvedValue({
      skills: [{ name: 'git-cloned', description: '来自 git clone', attachments: [], valid: true }],
    })
    waButtons(el)
      .find((b) => b.textContent?.trim() === 'Refresh')!
      .click()
    await settle(el)
    expect(el.shadowRoot!.querySelector('[data-testid="skill-preview"]')).toBeNull()
    el.remove()
  })

  it('renders the empty-store placeholder instead of a list when the store has no entries', async () => {
    apiMocks.skillsList.mockResolvedValue({ skills: [] })
    const el = await mount()
    await goto(el, 5)
    expect(el.shadowRoot!.querySelector('.provider-toolbar span.label')?.textContent).toContain(
      '0 skills in store',
    )
    const placeholder = el.shadowRoot!.querySelector('.prefs-placeholder')
    expect(placeholder).toBeTruthy()
    expect(placeholder!.textContent).toContain('The skill store is empty')
    expect(placeholder!.textContent).toContain('sebas skills add')
    expect(skillRows(el)).toHaveLength(0)
    el.remove()
  })

  it('sync failure renders the inline error callout instead of a result panel and unsets busy', async () => {
    const el = await mount()
    await goto(el, 5)
    apiMocks.skillsSync.mockRejectedValue(new ApiError(500, 'reconcile exploded'))
    waButtons(el)
      .find((b) => b.textContent?.trim() === 'Sync')!
      .click()
    await settle(el)

    const err = el.shadowRoot!.querySelector('[data-testid="skills-sync-error"]')
    expect(err).toBeTruthy()
    expect(err!.textContent).toContain('Sync failed')
    expect(err!.textContent).toContain('reconcile exploded')
    expect(el.shadowRoot!.querySelector('[data-testid="skills-sync-result"]')).toBeNull()
    // busy 复位：按钮恢复可点（否则一次失败就永久卡死 Sync）。
    const syncBtn = waButtons(el).find((b) => b.textContent?.trim() === 'Sync')!
    expect((syncBtn as unknown as HTMLButtonElement).disabled).toBe(false)
    el.remove()
  })

  it('preview surfaces a detail fetch failure inline instead of a skeleton', async () => {
    const el = await mount()
    await goto(el, 5)
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
    await goto(el, 5)
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
    await goto(el, 4)
    expect(el.section).toBe('agents')
    expect(apiMocks.agents).toHaveBeenCalledTimes(1)
    const native = el.shadowRoot!.querySelector('[data-testid="agent-row-native"]')
    expect(native).toBeTruthy()
    expect(native!.querySelector('[data-testid="agent-builtin-badge"]')?.textContent).toContain(
      'builtIn',
    )
    // native 行不得有编辑/删除入口。
    expect(native!.querySelector('button[title="Edit"]')).toBeNull()
    expect(native!.querySelector('button[title="Delete"]')).toBeNull()
    // store 行有编辑/删除入口。
    const row = agentRows(el).find((r) => r.dataset['id'] === 'claude')!
    expect(row.querySelector('button[title="Edit"]')).toBeTruthy()
    expect(row.querySelector('button[title="Delete"]')).toBeTruthy()
    el.remove()
  })

  it('create form submits the claude-shape payload with the typed id', async () => {
    apiMocks.agentsCreate.mockResolvedValue({ created: 'myclaude' })
    const el = await mount()
    await goto(el, 4)
    waButtons(el)
      .find((b) => b.textContent?.includes('New agent'))!
      .click()
    await settle(el)
    const dlg = dialogByLabel(el, 'New agent')
    setWaInput(el, 'Agent id', 'myclaude')
    // 形态下拉缺省 claude、路径预填 claude——直接保存。
    const save = waButtonsIn(dlg).find((b) => b.textContent?.trim() === 'Save')!
    save.click()
    await settle(el)
    expect(apiMocks.agentsCreate).toHaveBeenCalledWith('myclaude', {
      driver: 'claude',
      path: 'claude',
      display: null,
    })
    el.remove()
  })

  it('create form rejects the reserved native id without a request', async () => {
    const el = await mount()
    await goto(el, 4)
    waButtons(el)
      .find((b) => b.textContent?.includes('New agent'))!
      .click()
    await settle(el)
    setWaInput(el, 'Agent id', 'native')
    const dlg = dialogByLabel(el, 'New agent')
    waButtonsIn(dlg)
      .find((b) => b.textContent?.trim() === 'Save')!
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
    await goto(el, 4)
    agentRows(el)
      .find((r) => r.dataset['id'] === 'claude')!
      .querySelector<HTMLElement>('button[title="Edit"]')!
      .click()
    await settle(el)
    setWaInput(el, 'Display name (optional)', 'My Claude')
    const dlg = dialogByLabel(el, 'Edit agent claude')
    waButtonsIn(dlg)
      .find((b) => b.textContent?.trim() === 'Save')!
      .click()
    await settle(el)
    // 部分更新：只交 display（launch 定义保留存量——合并语义）。
    expect(apiMocks.agentsUpdate).toHaveBeenCalledWith('claude', { display: 'My Claude' })
    el.remove()
  })

  it('delete is a confirmed action and calls the DELETE endpoint on confirm', async () => {
    apiMocks.agentsDelete.mockResolvedValue({ deleted: 'claude' })
    const el = await mount()
    await goto(el, 4)
    agentRows(el)
      .find((r) => r.dataset['id'] === 'claude')!
      .querySelector<HTMLElement>('button[title="Delete"]')!
      .click()
    await settle(el)
    // 确认弹窗打开、请求未发。
    expect(apiMocks.agentsDelete).not.toHaveBeenCalled()
    const dlg = dialogByLabel(el, 'Delete agent')
    waButtonsIn(dlg)
      .find((b) => b.textContent?.trim() === 'Delete')!
      .click()
    await settle(el)
    expect(apiMocks.agentsDelete).toHaveBeenCalledWith('claude')
    // 删除后目录刷新 + 成功提示。
    expect(el.shadowRoot!.querySelector('[data-testid="agent-action"]')?.textContent).toContain(
      '已删除',
    )
    el.remove()
  })
})

describe('unify-router-process-shape：router 停止被拒的强制出口（D4）', () => {
  /** 装好带 router 行的 Services 分区，并在 confirm 弹窗里点掉 Disable。 */
  async function confirmDisableRouter(el: SebasSettingsModal): Promise<void> {
    await goto(el, 2)
    const routerCard = [...el.shadowRoot!.querySelectorAll<HTMLElement>('.service-card')].find(
      (c) => c.querySelector('.service-id')?.textContent === 'router',
    )!
    routerCard.querySelector<HTMLElement>('button[title="Disable service"]')!.click()
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
    expect(forceDialog(el).textContent).toContain('streaming')
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
    expect(el.shadowRoot!.textContent ?? '').toContain('status running')
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
    await goto(el, 1)
    const options = themeOptions(el)
    expect(options.map((b) => b.querySelector('.theme-option-label')?.textContent)).toEqual([
      'System',
      'Dark',
      'Light',
    ])
    expect(options[0]!.getAttribute('aria-pressed')).toBe('true')
    expect(options[1]!.getAttribute('aria-pressed')).toBe('false')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    el.remove()
  })

  it('choosing Light unsets wa-dark and persists sebas:theme=light', async () => {
    const el = await mount()
    await goto(el, 1)
    themeOptions(el)[2]!.click()
    await el.updateComplete
    expect(localStorage.getItem('sebas:theme')).toBe('light')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(false)
    expect(themeOptions(el)[2]!.getAttribute('aria-pressed')).toBe('true')
    expect(el.shadowRoot!.textContent).toContain('Applied immediately, saved for this browser.')
    el.remove()
  })

  it('choosing Dark sets wa-dark and persists; System returns to following the OS', async () => {
    const el = await mount()
    await goto(el, 1)
    themeOptions(el)[1]!.click()
    await el.updateComplete
    expect(localStorage.getItem('sebas:theme')).toBe('dark')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    // 回到 system：jsdom 无 matchMedia → 跟随解析为 dark。
    themeOptions(el)[0]!.click()
    await el.updateComplete
    expect(localStorage.getItem('sebas:theme')).toBe('system')
    expect(document.documentElement.classList.contains('wa-dark')).toBe(true)
    expect(el.shadowRoot!.textContent).toContain('Your OS currently asks for dark')
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
    overlay.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }))
    await el.updateComplete
    expect(el.open).toBe(false)
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
  await goto(el, 3)

  const status = el.shadowRoot?.querySelector('.provider-toolbar [role="status"]')
  expect(status?.textContent ?? '').toContain('no default set')
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
    await goto(el, 3)
    rowFor(el, name).querySelector<HTMLButtonElement>('button[title="Edit"]')!.click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.provider-editor') as HTMLElement
    expect(dialog).toBeTruthy()
    return dialog
  }

  /** 新建（preset/custom）编辑器。 */
  async function openCreateEditor(el: SebasSettingsModal, label: string): Promise<HTMLElement> {
    await goto(el, 3)
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
    await goto(el, 3)
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
    for (const label of ['New (preset)', 'New (custom)']) {
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
    await goto(el, 3)
    const newBtn = waButtons(el).find((b) => b.textContent?.includes('New (preset)'))!
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
    const select = dialog.querySelector('wa-select[label="Preset"]')
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
    await goto(el, 3)
    waButtons(el)
      .find((b) => b.textContent?.includes('New (preset)'))!
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
    expect(dialog.querySelector('wa-input[label="Name"]')).toBeNull()

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
    expect(dialog.querySelector('.model-entries .entries-label')?.textContent?.trim()).toBe('Models')
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
    await goto(el, 3)
    waButtons(el)
      .find((b) => b.textContent?.includes('New (custom)'))!
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

    setWaValue(dialog, 'wa-input[label="Name"]', 'my-api')
    // 协议缺省 OpenAI-compatible → 单个 Base URL 落 openai_chat 槽（D4）。
    setWaValue(dialog, 'wa-input[label="Base URL (OpenAI-compatible)"]', 'https://api.example/v1')
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

    setWaValue(dialog, 'wa-input[label="Name"]', 'my-api')
    setWaValue(dialog, 'wa-input[label="Base URL (OpenAI-compatible)"]', 'https://api.example/v1')
    // 展开后可编辑其余槽位（anthropic 槽 + responses 槽）与改名映射。
    setWaValue(dialog, 'wa-input[label="Base URL (Anthropic)"]', 'https://api.example/anthropic')
    setWaValue(
      dialog,
      'wa-input[label="Base URL (OpenAI Responses)"]',
      'https://api.example/responses',
    )
    setWaValue(
      dialog,
      'wa-input[label^="Model rename map"]',
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
    const index = navLabels(el).indexOf('Users')
    expect(index).toBeGreaterThanOrEqual(0)
    await goto(el, index)
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
      'Generic',
      'Appearance',
      'Services',
      'Users',
      'Models',
      'Agents',
      'Skills',
      'Env Vars',
      'About',
    ])
    rootEl.remove()

    const adminEl = await mountAs('admin')
    expect(navLabels(adminEl)).toEqual([
      'Generic',
      'Appearance',
      'Services',
      'Models',
      'Agents',
      'Skills',
      'Env Vars',
      'About',
    ])
    adminEl.remove()

    // member/viewer：无服务控制（services.control）、无用户管理（users.manage）。
    for (const role of ['member', 'viewer'] as const) {
      const el = await mountAs(role)
      const labels = navLabels(el)
      expect(labels).not.toContain('Users')
      expect(labels).not.toContain('Services')
      el.remove()
    }

    // role 缺省（服务端未启用鉴权的宿主）：既有分区全保留，Users 仍不可见
    // （users.manage 需要服务端身份）。
    const hostEl = await mountAs(null)
    expect(navLabels(hostEl)).toEqual([
      'Generic',
      'Appearance',
      'Services',
      'Models',
      'Agents',
      'Skills',
      'Env Vars',
      'About',
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
    expect(text).toContain('disabled')
    // created_at_unix → ISO 日期（列表不携带任何哈希字段可展示）。
    expect(text).toContain('created 2023-11-14')
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
      .find((b) => b.textContent?.includes('New user'))!
      .click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.user-create') as HTMLElement
    setWaValue(dialog, 'wa-input[label="Username"]', 'bob')
    setWaValue(dialog, 'wa-input[label^="Password"]', 'long-enough')
    pickWaSelect(dialog, 'wa-select[label="Role"]', 'admin')
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
      .find((b) => b.textContent?.includes('New user'))!
      .click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.user-create') as HTMLElement
    setWaValue(dialog, 'wa-input[label="Username"]', 'bob')
    setWaValue(dialog, 'wa-input[label^="Password"]', 'short')
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
      .find((b) => b.textContent?.includes('New user'))!
      .click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog.user-create') as HTMLElement
    setWaValue(dialog, 'wa-input[label="Username"]', 'alice')
    setWaValue(dialog, 'wa-input[label^="Password"]', 'long-enough')
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
    ;(userRow(el, 'root').querySelector('button[title="Disable user"]') as HTMLElement).click()
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
    ;(userRow(el, 'alice').querySelector('button[title="Reset password"]') as HTMLElement).click()
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
    ;(userRow(el, 'root').querySelector('button[title="Delete user"]') as HTMLElement).click()
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
  it('About Rust toolchain falls back to 未知 when the build-time inject is empty (4.3)', async () => {
    apiMocks.about.mockResolvedValue({
      uptime: '3h 12m',
      version: '0.4.2',
      rustc_version: '',
      router_listen: '127.0.0.1:8787',
      provider_count: 2,
      default_agent_kind: 'claude',
    })
    const el = await mount()
    await goto(el, 7)
    const row = [...el.shadowRoot!.querySelectorAll('dl.about-build .kv')].find((kv) =>
      kv.textContent?.includes('Rust toolchain'),
    )
    expect(row).toBeTruthy()
    // 构建期注入失败 = 「未知」，不再渲染空值行。
    expect(row!.querySelector('dd')!.textContent!.trim()).toBe('未知')
    el.remove()
  })

  it('About Rust toolchain shows the real value when present (4.3)', async () => {
    const el = await mount()
    await goto(el, 7)
    const row = [...el.shadowRoot!.querySelectorAll('dl.about-build .kv')].find((kv) =>
      kv.textContent?.includes('Rust toolchain'),
    )
    expect(row!.querySelector('dd')!.textContent!.trim()).toBe('1.88')
    el.remove()
  })

  it('the Services no-adapter banner keeps the sebas run command unbreakable (4.4)', async () => {
    apiMocks.adminServicesSafe.mockResolvedValue({ adapter_ok: false, services: [] })
    const el = await mount()
    await goto(el, 2)
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
