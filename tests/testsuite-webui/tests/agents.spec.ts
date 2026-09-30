/**
 * Journey — Settings → Agents 目录的 webui 全链路（add-agent-settings-and-
 * session-titles 的浏览器级收口：agent 目录此前只有 Rust 侧 mutation 测试与
 * 前端单测，「经 webui 表单添加 → 免重启进创建对话框 → 会话回合真完成」的
 * 完整链路没有浏览器级旅程）。
 *
 * 主案链路：
 *   Settings → Agents「＋ New agent」（claude 形态，二进制路径指 harness
 *   预构建的 fake-claude）→ POST /api/agents → 行即时呈现 + action callout
 *   → GET /api/agents catalog 同框 → 刷新后仍在（settings.db agents 表是
 *   目录唯一运行时权威，非前端内存态）→ 创建对话框 agent 下拉立即可选
 *   （免重启生效的可观察面）→ 经对话框创建会话（0-turn 占位，首条消息才
 *   spawn）→ composer 首条消息 → store 定义的 agent 解析出 fake-claude、
 *   回合真完成（done + 两侧气泡上屏 + detail.agent_kind 绑定）。
 *
 * 守卫案：保留 id "native" 的前端内联拒绝 + 同 id 重建的 409 就地呈现。
 * 删除案：行内 🗑 → 确认框 → 行即时消失、创建对话框不再提供——目录回归
 * 种子态，套件也借此自清理（agents 不在 resetState 的清场范围）。
 */
import { expect, test, type APIRequestContext } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'
import {
  ErrorCollector,
  FocusedSession,
  ProjectRail,
  SettingsModal,
  ensureSceneProject,
  getSession,
  listSessions,
  resetState,
  waitStatus,
} from './helpers'

const AGENT_ID = 'ui-fake'
const DISPLAY = 'UI Fake'

/** fake-claude 绝对路径：从 cwd 向上找仓库根的构建产物（harness 的
 * `cargo build --bin fake-claude` 已先于套件运行）；找不到就诚实报错，
 * 不静默退化成真 `claude`（那会让回合死于无凭据，链路假红）。 */
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
  throw new Error(
    `fake-claude not found under ${process.cwd()} — run via invoke testsuite-webui (it builds the fakes first)`,
  )
}

/** 轮询会话列表，等「agent_kind 绑定到指定 agent」的会话出现（创建对话框
 * confirm → 0-turn 占位入册；映射落 projects.db，rail/列表即时可见）。 */
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

test.describe.serial('Settings → Agents 目录 webui 全链路', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  test('webui 表单添加 claude agent，免重启进创建对话框并完成真实回合', async ({
    page,
    request,
  }) => {
    const rail = new ProjectRail(page)
    const detail = new FocusedSession(page)
    await resetState(request)
    const { name: projectName } = await ensureSceneProject(request)
    await page.goto('/')

    // 重试/复用场景的自愈（users-admin beforeAll 同款姿态）：agents 不在
    // resetState 清场范围，重跑前先清掉同名残留，保证「创建」一步可重入。
    expect((await request.delete(`/api/agents/${AGENT_ID}`)).status()).toBeLessThan(500)

    // 1) Settings → Agents：新建表单（Shape 缺省即 claude，无需再选）
    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    await settings.openSection('Agent')
    await expect(settings.panel.locator('[data-testid="agent-row-native"]')).toBeVisible()

    await settings.panel.locator('wa-button').filter({ hasText: '新建 agent' }).click()
    const dialog = page.locator('sebas-settings-modal wa-dialog[label="新建 agent"]')
    // wa-dialog 宿主在 top layer 读作 hidden——可见性断言落在渲染出的内部
    // 元素上（users-admin.spec 同款纪律）。
    await expect(dialog.locator('wa-button').filter({ hasText: '保存' })).toBeVisible()

    // wa-input 内层 input 逐字键入——`.fill` 会绕过 wa-input 的 input 事件
    // （addProjectByPath / users-admin 同款约束）。
    const idInput = dialog.locator('wa-input[label="Agent id"] input')
    await idInput.click()
    await idInput.pressSequentially(AGENT_ID)
    const displayInput = dialog.locator('wa-input[label="显示名（可选）"] input')
    await displayInput.click()
    await displayInput.pressSequentially(DISPLAY)
    // Binary path 有预填 'claude'——全选后覆盖输入，换成 fake-claude 绝对路径。
    const pathInput = dialog.locator('wa-input[label="二进制路径"] input')
    await pathInput.click()
    await pathInput.press('ControlOrMeta+a')
    await pathInput.pressSequentially(fakeClaudeBin())

    await dialog.locator('wa-button').filter({ hasText: '保存' }).click()
    await expect(
      page.locator('sebas-settings-modal [data-testid="agent-action"]'),
    ).toHaveText(`已创建 ${AGENT_ID}（免重启，创建会话下拉立即可选）`)
    const row = page.locator(
      `sebas-settings-modal [data-testid="agent-row"][data-id="${AGENT_ID}"]`,
    )
    await expect(row).toBeVisible()
    await expect(row.locator('.provider-badge')).toHaveText(DISPLAY)
    await expect(row.locator('.provider-key')).toHaveText('可达')

    // 2) API 同框：catalog 已含条目（同一 state snapshot 的读面）。
    const catalog = (await (await request.get('/api/agents')).json()) as {
      agents: Array<{ id: string; display: string; reachable: boolean }>
    }
    const entry = catalog.agents.find((a) => a.id === AGENT_ID)
    expect(entry).toBeTruthy()
    expect(entry?.reachable).toBe(true)
    expect(entry?.display).toBe(DISPLAY)

    // 3) 刷新后仍在——目录落库，不是前端内存态。
    await page.reload()
    await settings.openViaSidebar()
    await settings.openSection('Agent')
    await expect(
      page.locator(`sebas-settings-modal [data-testid="agent-row"][data-id="${AGENT_ID}"]`),
    ).toBeVisible()
    await settings.close()

    // 4) 创建对话框即时可选（免重启生效的可观察面；reachable → 可选不禁用）。
    await rail.openNewSessionDialog(projectName)
    const agentSelect = rail.newSessionDialog().locator('[data-testid="dialog-agent-select"]')
    const optionValues = await agentSelect
      .locator('wa-option')
      .evaluateAll((els) => els.map((el) => el.getAttribute('value')))
    expect(optionValues).toContain(AGENT_ID)
    await expect(agentSelect.locator(`wa-option[value="${AGENT_ID}"]`)).not.toHaveAttribute(
      'disabled',
    )
    await rail.pickDialogAgent(AGENT_ID)
    await rail.confirmNewSessionDialog()
    await expect(rail.newSessionDialog()).toBeHidden()

    // 5) 0-turn 占位会话已入册（agent_kind 绑定），首条消息才 spawn。
    const key = await waitForAgentSession(request, AGENT_ID)
    if (!key) throw new Error('dialog confirm 未产生绑定 ui-fake 的会话')
    const bound = await getSession(request, key)
    expect(bound.detail?.agent_kind).toBe(AGENT_ID)

    // 6) 完整链路的最后一环：store 定义的 agent 解析出 fake-claude，回合真完成。
    await page.goto(`/sessions/${key}`)
    await expect(detail.sessionHead).toBeVisible()
    await detail.sendFollowUp('hello')
    await waitStatus(request, key, ['done'])
    await expect(detail.userTurn('hello')).toBeVisible()
    await expect(detail.agentTurn('world').first()).toBeVisible({ timeout: 15_000 })
  })

  test('保留 id 与重复 id 在表单内联拒绝', async ({ page }) => {
    await page.goto('/')
    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    await settings.openSection('Agent')

    await settings.panel.locator('wa-button').filter({ hasText: '新建 agent' }).click()
    const dialog = page.locator('sebas-settings-modal wa-dialog[label="新建 agent"]')
    const save = dialog.locator('wa-button').filter({ hasText: '保存' })
    await expect(save).toBeVisible()

    // 保留 id：前端内联拒绝（不发请求——spec「内置 native kernel 不落表、
    // 不可写」，提交前就拦住）。
    const idInput = dialog.locator('wa-input[label="Agent id"] input')
    await idInput.click()
    await idInput.pressSequentially('native')
    await save.click()
    await expect(dialog.locator('[data-testid="agent-form-error"]')).toHaveText(
      '"native" 是内置内核保留 id，不可占用',
    )

    // 重复 id：服务端 409 → ApiError 文案就地呈现在同一表单内。
    await idInput.press('ControlOrMeta+a')
    await idInput.pressSequentially(AGENT_ID)
    await save.click()
    await expect(dialog.locator('[data-testid="agent-form-error"]')).toContainText('已存在')
    await dialog.locator('wa-button').filter({ hasText: '取消' }).click()
    await expect(dialog).toBeHidden()
  })

  test('删除即时收场：行消失且创建对话框不再提供', async ({ page, request }) => {
    const rail = new ProjectRail(page)
    await resetState(request)
    const { name: projectName } = await ensureSceneProject(request)
    await page.goto('/')

    const settings = new SettingsModal(page)
    await settings.openViaSidebar()
    await settings.openSection('Agent')
    const row = page.locator(
      `sebas-settings-modal [data-testid="agent-row"][data-id="${AGENT_ID}"]`,
    )
    await row.locator('button[title="删除"]').click()
    const confirm = page.locator('sebas-settings-modal wa-dialog[label="删除 agent"]')
    await expect(confirm.locator('wa-button').filter({ hasText: '删除' })).toBeVisible()
    await confirm.locator('wa-button').filter({ hasText: '删除' }).click()
    await expect(
      page.locator('sebas-settings-modal [data-testid="agent-action"]'),
    ).toHaveText(`已删除 ${AGENT_ID}（已建会话继续到自然结束）`)
    await expect(row).toHaveCount(0)
    await settings.close()

    // 消费面即时收场：创建对话框的 catalog 即取即用，不再提供已删 id。
    await rail.openNewSessionDialog(projectName)
    const optionValues = await rail
      .newSessionDialog()
      .locator('[data-testid="dialog-agent-select"] wa-option')
      .evaluateAll((els) => els.map((el) => el.getAttribute('value')))
    expect(optionValues).not.toContain(AGENT_ID)
    await rail.cancelNewSessionDialog()
  })
})
