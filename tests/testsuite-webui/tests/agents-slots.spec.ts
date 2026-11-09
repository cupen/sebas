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
  pickSelectOption,
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
    const native = settings.panel.getByText('native', { exact: true }).locator('xpath=../..')
    await expect(native).toBeVisible()
    await expect(native.getByRole('button', { name: /编\s*辑/ })).toHaveCount(0)
    await expect(native.getByRole('button', { name: /删\s*除/ })).toHaveCount(0)
    await expect(native.getByRole('button', { name: /安\s*装/ })).toHaveCount(0)

    // 已装 store 行：可达状态 + 编辑 + 删除（config 种子 claude 行）。
    const claude = settings.panel.getByText('claude', { exact: true }).locator('xpath=../..')
    await expect(claude).toContainText('可达')
    await expect(claude.getByRole('button', { name: /编\s*辑/ })).toBeVisible()
    await expect(claude.getByRole('button', { name: /删\s*除/ })).toHaveCount(1)

    // 未装配方行与已装行同列（统一目录，无独立配方区块）：至少一条配方行
    // 带「未安装」徽标与安装按钮。
    const uninstalled = settings.panel.getByText('未安装', { exact: true })
    const recipeCount = await uninstalled.count()
    expect(recipeCount).toBeGreaterThanOrEqual(1)
    const firstRecipe = uninstalled.first().locator('xpath=../..')
    await expect(firstRecipe.getByRole('button', { name: /安\s*装/ })).toBeVisible()

    // 同列性：配方行与 agent 行同处设置面板的目录区。
    await expect(uninstalled).toHaveCount(recipeCount)

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
    const row = settings.panel.getByText(AGENT_ID, { exact: true }).locator('xpath=../..')
    await expect(row).toBeVisible({ timeout: 10_000 })
    await row.getByRole('button', { name: /编\s*辑/ }).click()
    const dialog = page.getByRole('dialog', { name: `编辑 agent：${AGENT_ID}` })
    await expect(dialog.getByRole('button', { name: /保\s*存/ })).toBeVisible()

    // 档位段在场：四档行（opus/sonnet/haiku/fable）+ 目录模型可选（catalog
    // 异步拉取）。行以 group 语义命名，档位内是启用勾选 + 模型下拉 +
    // 默认单选。
    const slots = dialog.getByRole('group', { name: /档位/ })
    await expect(slots).toBeVisible()
    const slotRow = (slot: string) =>
      slots.getByRole('group', { name: new RegExp(slot, 'i') })
    await expect(slotRow('opus')).toBeVisible()
    await expect(slotRow('sonnet')).toBeVisible()
    await expect(slotRow('haiku')).toBeVisible()
    await expect(slotRow('fable')).toBeVisible()
    await expect
      .poll(async () => {
        await slotRow('opus').getByRole('combobox', { name: /模型/ }).click()
        const options = page.locator(
          '.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option',
        )
        const texts = await options.allTextContents()
        await page.keyboard.press('Escape')
        return texts.join('\n')
      })
      .toContain('m-opus')

    // 启用 opus/sonnet/haiku 并各配模型；fable 保持停用（停用档不出键的 UI 面）。
    for (const [slot, model] of [
      ['opus', 'm-opus'],
      ['sonnet', 'm-sonnet'],
      ['haiku', 'm-haiku'],
    ] as const) {
      const target = slotRow(slot)
      await target.getByRole('checkbox', { name: /启用/ }).check()
      await pickSelectOption(
        page,
        target.getByRole('combobox', { name: /模型/ }),
        model,
      )
    }
    // 默认档 = sonnet（native radio）。
    await slotRow('sonnet').getByRole('radio', { name: /默认/ }).check()

    await dialog.getByRole('button', { name: /保\s*存/ }).click()
    await expect(settings.panel.getByRole('alert')).toContainText(`已更新 ${AGENT_ID}`)

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
    await row.getByRole('button', { name: /编\s*辑/ }).click()
    await expect(dialog.getByRole('button', { name: /保\s*存/ })).toBeVisible()
    await expect(slotRow('opus').getByRole('checkbox', { name: /启用/ })).toBeChecked()
    await expect(slotRow('fable').getByRole('checkbox', { name: /启用/ })).not.toBeChecked()
    // AntD 6 把选中标签渲染为 .ant-select-content 的文本；combobox 的
    // 内部 input value 恒为空，不能作为回填断言面。
    await expect(
      slotRow('haiku').locator('.ant-select-content').filter({ hasText: 'm-haiku' }),
    ).toBeVisible()
    await expect(slotRow('sonnet').getByRole('radio', { name: /默认/ })).toBeChecked()
    await dialog.getByRole('button', { name: /取\s*消/ }).click()
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
    const dialog = rail.newSessionDialog()
    const agentSelect = dialog.getByRole('combobox', { name: 'Agent' })
    // 目录异步拉取：展开后等 slot-claude 进清单。
    await agentSelect.click()
    const agentOption = page
      .locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option')
      .filter({ hasText: new RegExp(`^${AGENT_ID}$`) })
    await expect(agentOption).toBeVisible({ timeout: 10_000 })
    await agentOption.click()

    // claude agent → 单层档位下拉：default + 启用档（映射存在，fable 不在），
    // 预选 = 映射默认档 sonnet（跳过 last-used 记忆的 UI 面）。
    const slotSelect = dialog.getByRole('combobox', { name: /档位/ })
    await expect(slotSelect).toBeVisible()
    await slotSelect.click()
    const slotOptions = page.locator(
      '.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option',
    )
    await expect(slotOptions).toHaveCount(4)
    const slotTexts = await slotOptions.allTextContents()
    expect(slotTexts.map((text) => text.trim())).toEqual(['default', 'opus', 'sonnet', 'haiku'])
    await page.keyboard.press('Escape')
    await expect(
      dialog.locator('.ant-select-content').filter({ hasText: 'sonnet' }),
    ).toBeVisible()
    // provider 两级目录在 claude agent 下不渲染。
    await expect(dialog.getByRole('combobox', { name: /provider/i })).toHaveCount(0)

    // 非 claude agent（native 槽位，展示名 PI）→ 档位下拉让位 provider/模型
    // 目录。可见选项行是 AntD 的 .ant-select-item-option（其文案为展示名 PI）；
    // role=option 命中的是 rc-select 挂在 combobox 内的隐藏 a11y 镜像（文本是
    // 值 native），不可点。业务断言：选中内置 PI 槽位。
    await agentSelect.click()
    const agentDropdown = page.locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)')
    const nativeOption = agentDropdown
      .locator('.ant-select-item-option')
      .filter({ hasText: /^PI$/ })
    await expect(nativeOption).toBeVisible()
    await nativeOption.click()
    await expect(dialog.getByRole('combobox', { name: /档位/ })).toHaveCount(0)
    await expect(dialog.getByRole('combobox', { name: '模型' })).toBeVisible()

    // 回到 claude agent、改选 opus、确认创建：0-turn 会话入册（model=opus
    // 随创建载荷，spawn 后 SetModel 链路由 e2e 旅程覆盖）。
    await agentSelect.click()
    await agentOption.click()
    await expect(slotSelect).toBeVisible()
    await pickSelectOption(page, slotSelect, 'opus')
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
