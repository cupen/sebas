/**
 * Journey — Settings → Agents 统一目录与 claude 档位映射的浏览器级收口
 * （add-claude-model-slots 4.1/4.2/4.3/5.3）。
 *
 * 三条旅程：
 *   1. 统一目录形态：一行一 agent（native 内置行无写按钮；已装行「已安装」
 *      徽标 + 齿轮 + 删除；未装配方行与已装行同列，行内「安装」按钮——
 *      独立配方区块不再存在）。
 *   2. claude 档位映射闭环：表单四档行（checkbox + 目录选择器 + 默认
 *      radio）配置 → 保存 → /api/agents 回填同框 → 重开表单预填。
 *   3. new-session 联动：选中 claude agent → 单层档位下拉（default + 启用
 *      档，预选 agent 默认档，跳过 last-used 记忆）；非 claude agent →
 *      既有 provider 目录；确认创建 0-turn 会话入册。
 *
 * 专用 agent（slot-claude）与 provider（slotprov）随旅程 API 自建自清，
 * 不污染种子态（编辑种子 claude 行会把 store 行翻 source=ui，遮蔽 config
 * 种子——测试隔离纪律）。
 */
import { expect, test, type APIRequestContext } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'
import {
  ErrorCollector,
  ProjectRail,
  SettingsModal,
  ensureSceneProject,
  getSession,
  listSessions,
  resetState,
} from './helpers'

const AGENT_ID = 'slot-claude'
const PROVIDER = 'slotprov'

/** fake-claude 绝对路径（与 agents.spec 同款；harness 已先构建）。 */
function fakeClaudeBin(): string {
  const bin = `fake-claude${process.platform === 'win32' ? '.exe' : ''}`
  let dir = process.cwd()
  for (let depth = 0; depth < 6; depth++) {
    const candidate = path.join(dir, 'target', 'debug', bin)
    if (fs.existsSync(candidate)) return candidate
    const parent = path.dirname(dir)
    if (parent === dir) break
    dir = parent
  }
  throw new Error(`fake-claude not found under ${process.cwd()} — run via invoke testsuite-webui`)
}

/** wa-select / wa-checkbox 没有 native 表单语义——设值后派发 change（套件
 * 既有 pickDialogAgent 同款纪律）。 */
async function setWaValue(
  el: { evaluate: (fn: unknown, arg?: unknown) => Promise<unknown> },
  value: string,
): Promise<void> {
  await el.evaluate(
    (e: unknown, v: string) => {
      const node = e as unknown as { value: string }
      node.value = v
      node.dispatchEvent(new Event('change', { bubbles: true }))
    },
    value,
  )
}

/** wa-checkbox 勾选（ev.target.checked 语义）。 */
async function setWaChecked(
  el: { evaluate: (fn: unknown, arg?: unknown) => Promise<unknown> },
  checked: boolean,
): Promise<void> {
  await el.evaluate(
    (e: unknown, c: boolean) => {
      const node = e as unknown as { checked: boolean }
      node.checked = c
      node.dispatchEvent(new Event('change', { bubbles: true }))
    },
    checked,
  )
}

/** 轮询会话列表，等「agent_kind 绑定到指定 agent」的 0-turn 占位入册。 */
async function waitForAgentSession(
  request: APIRequestContext,
  agent: string,
): Promise<string | null> {
  let found: string | null = null
  await expect
    .poll(async () => {
      const rows = await listSessions(request)
      const row = rows.find((r) => r.agent_kind === agent)
      found = row?.encoded_key ?? null
      return found
    })
    .not.toBeNull()
  return found
}

test.describe.serial('Settings → Agents 统一目录与 claude 档位映射', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  test('统一目录：一行一 agent + 安装状态 + 未装配方行内安装入口', async ({
    page,
  }) => {
    await page.goto('/')
    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    await settings.openSection('Agent')

    // native 内置行：恒在、无写按钮（不可编辑/删除/安装）。
    const native = settings.panel.locator('[data-testid="agent-row-native"]')
    await expect(native).toBeVisible()
    await expect(native.locator('[data-testid="agent-gear"]')).toHaveCount(0)
    await expect(native.locator('button[title="删除"]')).toHaveCount(0)
    await expect(native.locator('[data-testid="install-button"]')).toHaveCount(0)

    // 已装 store 行：「已安装」徽标 + 齿轮 + 删除（config 种子 claude 行）。
    const claude = settings.panel.locator(
      `[data-testid="agent-row"][data-id="claude"]`,
    )
    await expect(claude.locator('[data-testid="install-status"]')).toHaveText('已安装')
    await expect(claude.locator('[data-testid="agent-gear"]')).toBeVisible()
    await expect(claude.locator('button[title="删除"]')).toHaveCount(1)

    // 未装配方行与已装行同列（统一目录，无独立配方区块）：至少一条配方行
    // 带安装按钮（opencode / pi 在套件环境未装）。
    const recipes = settings.panel.locator('[data-testid="install-recipe"]')
    const recipeCount = await recipes.count()
    expect(recipeCount).toBeGreaterThanOrEqual(1)
    const first = recipes.first()
    await expect(first.locator('[data-testid="install-status"]')).toHaveText('未安装')
    await expect(first.locator('[data-testid="install-button"]')).toBeVisible()

    // 同列性：配方行与 agent 行同处 agents-list 容器。
    await expect(
      settings.panel.locator(
        `[data-testid="agents-list"] [data-testid="install-recipe"]`,
      ),
    ).toHaveCount(recipeCount)

    await settings.close()
  })

  test('claude 档位映射：表单配置 → 保存 → API 同框 → 预填', async ({
    page,
    request,
  }) => {
    await resetState(request)
    // 重跑自愈：清掉同名残留。
    expect((await request.delete(`/api/agents/${AGENT_ID}`)).status()).toBeLessThan(500)
    expect((await request.delete(`/api/providers/${PROVIDER}`)).status()).toBeLessThan(500)

    // 目录源：一个带三个模型的自定义 provider（127.0.0.1 段哑地址，不外呼）。
    const prov = await request.post('/api/providers', {
      data: {
        name: PROVIDER,
        base_url_openai_chat: 'http://127.0.0.1:9/v1',
        models: [{ id: 'm-opus' }, { id: 'm-sonnet' }, { id: 'm-haiku' }],
      },
    })
    expect(prov.ok()).toBe(true)
    const created = await request.post('/api/agents', {
      data: { id: AGENT_ID, driver: 'claude', path: fakeClaudeBin() },
    })
    expect(created.ok()).toBe(true)

    await page.goto('/')
    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    await settings.openSection('Agent')
    const row = settings.panel.locator(
      `[data-testid="agent-row"][data-id="${AGENT_ID}"]`,
    )
    await expect(row).toBeVisible({ timeout: 10_000 })
    await row.locator('[data-testid="agent-gear"]').click()
    const dialog = page.locator(
      `sebas-settings-modal wa-dialog[label="编辑 agent ${AGENT_ID}"]`,
    )
    await expect(
      dialog.locator('wa-button').filter({ hasText: '保存' }),
    ).toBeVisible()

    // 档位段在场：四档行 + 目录模型可选（catalog 异步拉取）。wa-option 在
    // 下拉未展开时是 hidden——断言附着（属性读取）而非可见（agents.spec
    // 的 dialog-agent-select 同款纪律）。
    const slots = dialog.locator('[data-testid="agent-form-slots"]')
    await expect(slots).toBeVisible()
    await expect(
      dialog.locator('[data-testid="agent-slot-row"]'),
    ).toHaveCount(4)
    const opusRow = dialog.locator('[data-testid="agent-slot-row"][data-slot="opus"]')
    await expect
      .poll(() =>
        opusRow
          .locator('[data-testid="agent-slot-model"] wa-option')
          .evaluateAll((els) => els.map((el) => el.getAttribute('value'))),
      )
      .toContain('m-opus')

    // 启用 opus/sonnet/haiku 并各配模型；fable 保持停用（停用档不出键的 UI 面）。
    const slotRow = (slot: string) =>
      dialog.locator(`[data-testid="agent-slot-row"][data-slot="${slot}"]`)
    for (const [slot, model] of [
      ['opus', 'm-opus'],
      ['sonnet', 'm-sonnet'],
      ['haiku', 'm-haiku'],
    ] as const) {
      await setWaChecked(slotRow(slot).locator('[data-testid="agent-slot-enabled"]'), true)
      await setWaValue(slotRow(slot).locator('[data-testid="agent-slot-model"]'), model)
    }
    // 默认档 = sonnet（native radio）。
    await slotRow('sonnet').locator('[data-testid="agent-slot-default"]').check()

    await dialog.locator('wa-button').filter({ hasText: '保存' }).click()
    await expect(
      page.locator('sebas-settings-modal [data-testid="agent-action"]'),
    ).toContainText(`已更新 ${AGENT_ID}`)

    // API 同框：GET /api/agents 回填原始映射（含 default）。
    const catalog = (await (await request.get('/api/agents')).json()) as {
      agents: Array<{ id: string; models?: Record<string, string> }>
    }
    const entry = catalog.agents.find((a) => a.id === AGENT_ID)
    expect(entry?.models).toEqual({
      opus: 'm-opus',
      sonnet: 'm-sonnet',
      haiku: 'm-haiku',
      default: 'sonnet',
    })

    // 预填：重开表单，checkbox/选择器/默认 radio 全量回填。
    await row.locator('[data-testid="agent-gear"]').click()
    await expect(
      dialog.locator('wa-button').filter({ hasText: '保存' }),
    ).toBeVisible()
    // wa-checkbox 的 checked 是 lit property 绑定（?checked=）——断言落在
    // property 上，attribute 恒不在场。
    await expect
      .poll(() =>
        slotRow('opus')
          .locator('[data-testid="agent-slot-enabled"]')
          .evaluate((e) => (e as unknown as { checked: boolean }).checked),
      )
      .toBe(true)
    await expect
      .poll(() =>
        slotRow('fable')
          .locator('[data-testid="agent-slot-enabled"]')
          .evaluate((e) => (e as unknown as { checked: boolean }).checked),
      )
      .toBe(false)
    await expect(
      slotRow('haiku').locator('[data-testid="agent-slot-model"]'),
    ).toHaveAttribute('value', 'm-haiku')
    await expect(
      slotRow('sonnet').locator('[data-testid="agent-slot-default"]'),
    ).toBeChecked()
    await dialog.locator('wa-button').filter({ hasText: '取消' }).click()
    await settings.close()
  })

  test('new-session 联动：claude agent 档位下拉预选默认档，非 claude 回 provider 目录', async ({
    page,
    request,
  }) => {
    await resetState(request)
    const { name: projectName } = await ensureSceneProject(request)
    await page.goto('/')

    const rail = new ProjectRail(page)
    await rail.openNewSessionDialog(projectName)
    // 目录异步拉取：等 slot-claude 进 agent 下拉。
    const agentSelect = rail.newSessionDialog().locator('[data-testid="dialog-agent-select"]')
    await expect
      .poll(async () =>
        agentSelect
          .locator('wa-option')
          .evaluateAll((els) => els.map((el) => el.getAttribute('value'))),
      )
      .toContain(AGENT_ID)

    // claude agent → 单层档位下拉：default + 启用档（映射存在，fable 不在），
    // 预选 = 映射默认档 sonnet（跳过 last-used 记忆的 UI 面）。
    await rail.pickDialogAgent(AGENT_ID)
    const slotSelect = rail
      .newSessionDialog()
      .locator('[data-testid="dialog-slot-select"]')
    await expect(slotSelect).toBeVisible()
    expect(
      await slotSelect
        .locator('wa-option')
        .evaluateAll((els) => els.map((el) => el.getAttribute('value'))),
    ).toEqual(['default', 'opus', 'sonnet', 'haiku'])
    await expect(slotSelect).toHaveAttribute('value', 'sonnet')
    // provider 两级目录在 claude agent 下不渲染。
    await expect(
      rail.newSessionDialog().locator('[data-testid="dialog-provider-select"]'),
    ).toHaveCount(0)

    // 非 claude agent（native = pi driver）→ 档位下拉让位 provider 目录。
    await rail.pickDialogAgent('native')
    await expect(
      rail.newSessionDialog().locator('[data-testid="dialog-slot-select"]'),
    ).toHaveCount(0)
    await expect(
      rail.newSessionDialog().locator('[data-testid="dialog-provider-select"]'),
    ).toBeVisible()

    // 回到 claude agent、改选 opus、确认创建：0-turn 会话入册（model=opus
    // 随创建载荷，spawn 后 SetModel 链路由 e2e 旅程覆盖）。
    await rail.pickDialogAgent(AGENT_ID)
    await expect(slotSelect).toBeVisible()
    await setWaValue(slotSelect, 'opus')
    await rail.confirmNewSessionDialog()
    await expect(rail.newSessionDialog()).toBeHidden()
    const key = await waitForAgentSession(request, AGENT_ID)
    if (!key) throw new Error('dialog confirm 未产生绑定 slot-claude 的会话')
    const bound = await getSession(request, key)
    expect(bound.detail?.agent_kind).toBe(AGENT_ID)

    // 清理：专用 agent 与 provider 随旅程离场（种子态不残留）。
    expect((await request.delete(`/api/agents/${AGENT_ID}`)).status()).toBeLessThan(500)
    expect((await request.delete(`/api/providers/${PROVIDER}`)).status()).toBeLessThan(500)
  })
})
