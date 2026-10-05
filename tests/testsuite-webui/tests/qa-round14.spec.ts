/**
 * Journey — fix-webui-qa-round14 的浏览器半边（webui / agent-workbench 增量）。
 * Runs in the MAIN (auth-off) suite, port 9899. 单测已有的组件半边之外，这里
 * 以真实后端 + 真渲染钉四件只有浏览器能证明的事：
 *
 *  1. 「Session creation rejections are surfaced」：创建提交被类型化拒绝
 *     （容量满文案形状）→ notice 层 toast + 对话框内联错误就地呈现，浏览器
 *     留在工作台、无幻影 /sessions/<key> URL。容量=32 在沙箱不可达，用
 *     Playwright route 拦截返回后端 `rejection_response` 的真实错误形状
 *     （{"error": SessionRejection Display}）——GUI 黑盒只关心呈现半边，
 *     拒绝文案本身是后端单测钉过的字面量。
 *  2. 「Model aliases are first-class in model selection surfaces」：API 播种
 *     provider + 别名 `opus`（与 fake-claude 会话模型同名——选中即以别名串
 *     为模型值，走既有 set_model 路径到真驱动，不必桩认识新名字）：
 *     - composer 模型菜单：别名组置顶 + 来源徽标，点击别名 → 真实切换；
 *     - 创建弹窗模型下拉：别名选项（「别名 · 名」）在列；
 *     - 别名编辑器目标 provider 下拉：store 行 + config.toml 种子行
 *       （「anthropic（config 种子）」标注，沙箱 config 自带该种子）。
 *  3. 「History group expanded state persists」：展开 → reload 仍展开；
 *     收起 → reload 仍收起（缺省收起，真 localStorage + 真刷新）。
 *  4. 「Creation dialog accepts an optional session title」：带 title 创建 →
 *     rail 行即显该名（创建后立即重命名兜底链，免单独改名步骤）。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  getSession,
  ProjectRail,
  resetState,
  waitStatus,
} from './helpers/index'

let collector: ErrorCollector
test.beforeEach(({ page }) => {
  collector = new ErrorCollector(page)
})
test.afterEach(() => {
  expect(collector.pageErrors).toEqual([])
  expect(collector.consoleErrors).toEqual([])
})

const CAPACITY_MESSAGE = '会话数已达上限 32'

test.describe('创建失败的类型化拒绝呈现（round14 2.1，webui delta）', () => {
  test('容量拒绝：notice 上屏 + 对话框就地报错 + 无幻影会话 URL', async ({ page, request }) => {
    await resetState(request)
    await ensureSceneProject(request)

    // 拦截创建 POST：返回后端容量拒绝的真实错误形状（400 + {"error"}）。
    await page.route('**/api/sessions', async (route) => {
      if (route.request().method() !== 'POST') return route.continue()
      await route.fulfill({
        status: 400,
        contentType: 'application/json',
        body: JSON.stringify({ error: CAPACITY_MESSAGE }),
      })
    })

    await page.goto('/')
    const rail = new ProjectRail(page)
    const { name: projectName } = await ensureSceneProject(request)
    await rail.ensureProjectExpanded(projectName)
    await rail.openNewSessionDialog(projectName)
    await rail.pickDialogAgent('claude')
    await rail.confirmNewSessionDialog()

    // 类型化文案经 notice 层上屏（操作者在动作点上得到显式失败反馈）。
    await expect(
      page.locator('wa-toast-item').filter({ hasText: CAPACITY_MESSAGE }),
    ).toBeVisible({ timeout: 10_000 })
    // 对话框保持打开、内联错误就地呈现同一文案（不假装创建成功）。
    // 注意 wa-dialog 的宿主元素在 top layer 里读作 hidden——可见性断言
    // 落在渲染内容（heading / 内联错误）上，与 ProjectRail helper 同纪律。
    const dialog = rail.newSessionDialog()
    await expect(
      dialog.locator('h2, [role="heading"]').first(),
    ).toBeVisible({ timeout: 10_000 })
    await expect(dialog.locator('[data-testid="dialog-error"]')).toContainText(
      CAPACITY_MESSAGE,
      { timeout: 10_000 },
    )
    // 无幻影 URL：浏览器留在工作台，历史里没有 /sessions/<key>。
    expect(new URL(page.url()).pathname).toBe('/')

    await page.unroute('**/api/sessions')
  })
})

test.describe('模型别名的一等消费面（round14 3.1/3.2/3.3，webui delta）', () => {
  const PROVIDER = 'r14-alias-provider'
  const ALIAS = 'opus' // 与 fake-claude 会话模型同名：选中即真切换（桩认识 opus）。

  test.beforeEach(async ({ request }) => {
    // 幂等播种：store provider 行 + 别名（与 provider-gate 同纪律）。
    await request.delete(`/api/providers/${PROVIDER}`)
    await request.delete(`/api/model-aliases/${ALIAS}`)
    const provider = await request.post('/api/providers', {
      data: { name: PROVIDER, api_key: 'sk-r14-alias-dummy' },
    })
    expect([200, 201], `seed provider: ${provider.status()}`).toContain(provider.status())
    const alias = await request.post('/api/model-aliases', {
      data: { alias: ALIAS, provider: PROVIDER },
    })
    expect([200, 201], `seed alias: ${alias.status()}`).toContain(alias.status())
  })

  test.afterEach(async ({ request }) => {
    await request.delete(`/api/model-aliases/${ALIAS}`)
    await request.delete(`/api/providers/${PROVIDER}`)
  })

  test('composer 模型菜单：别名组置顶带来源徽标，点击别名以别名为模型值真实切换', async ({
    page,
    request,
  }) => {
    await resetState(request)
    const key = await createSession(request, { prompt: 'alias-composer' })
    await waitStatus(request, key, ['done'])

    await page.goto(`/sessions/${key}`)
    const chip = page.locator('sebas-workbench-composer [data-testid="model-chip"]')
    await expect(chip).toBeVisible({ timeout: 15_000 })

    await chip.click()
    // 「别名」组置顶；同名冲突时别名条目赢（目录 opus 被折叠，只出现一次）。
    const menu = page.locator('sebas-workbench-composer [data-testid="model-menu"]')
    const aliasItem = menu.locator('.menu-item[data-model="opus"]')
    const aliasTag = aliasItem.locator('[data-testid="model-alias-tag"]')
    await expect(aliasTag).toBeVisible({ timeout: 10_000 })
    await expect(aliasTag).toHaveText('别名')
    // 徽标悬浮说明点名目标 provider（D-4-1 来源可辨）。
    await expect(aliasItem).toHaveAttribute('title', new RegExp(PROVIDER))

    // 选中别名：以别名串为模型值走既有 set_model 路径（fake-claude 认识
    // opus）——current_model 跟随、芯片呈现。
    await aliasItem.click()
    await expect
      .poll(async () => (await getSession(request, key)).detail?.current_model, {
        timeout: 15_000,
        intervals: [250],
      })
      .toBe('opus')
    await expect(chip).toContainText('opus', { timeout: 15_000 })
  })

  test('创建弹窗模型下拉：别名短名与目录模型并排（来源标注）', async ({ page, request }) => {
    await resetState(request)
    await page.goto('/')
    const rail = new ProjectRail(page)
    const { name: projectName } = await ensureSceneProject(request)
    await rail.ensureProjectExpanded(projectName)
    await rail.openNewSessionDialog(projectName)

    // 目录模型为空但别名在场：不再落「尚未配置」引导，模型下拉可用且含
    // 别名条目（选项文案点名「别名」）。wa-option 在收起的下拉弹层里读作
    // hidden——断言「在 DOM」与文本/属性，而不是可见性。
    const modelSelect = rail
      .newSessionDialog()
      .locator('[data-testid="dialog-model-select"]')
    const aliasOption = modelSelect.locator(`wa-option[value="${ALIAS}"]`)
    await expect(aliasOption).toHaveCount(1, { timeout: 10_000 })
    expect(await aliasOption.textContent()).toContain('别名')
    await expect(aliasOption).toHaveAttribute('title', new RegExp(PROVIDER))
    // 下拉未禁用（别名本身就是可用取值）。
    await expect(modelSelect).not.toHaveAttribute('disabled')
  })

  test('别名编辑器目标 provider 下拉：store 行 + config 种子行（标注来源）', async ({
    page,
  }) => {
    await page.goto('/')
    const settings = page.locator('sebas-settings-modal .panel[role="dialog"][aria-label="设置"]')
    await page.locator('sebas-app .sidebar-footer button[aria-label="打开设置"]').click()
    await expect(settings).toBeVisible()
    await settings.locator('.nav-item', { hasText: '别名' }).click()

    const aliases = page.locator('sebas-model-aliases')
    await expect(aliases.locator('[data-testid="alias-create"]')).toBeVisible({
      timeout: 10_000,
    })
    await aliases.locator('[data-testid="alias-create"]').click()

    const targetSelect = aliases.locator('[data-testid="alias-provider-select"]')
    // store 行在前……（wa-option 在收起的弹层里读作 hidden：断言「在 DOM」
    // 与文本/属性，而非可见性。）
    const storeOption = targetSelect.locator(`wa-option[value="${PROVIDER}"]`)
    await expect(storeOption).toHaveCount(1, { timeout: 10_000 })
    // ……config.toml 种子行补后（沙箱 config 自带 [provider.anthropic]），
    // 选项文案标注来源（D-4-2）。
    const seedOption = targetSelect.locator('wa-option[value="anthropic"]')
    await expect(seedOption).toHaveCount(1)
    expect(await seedOption.textContent()).toContain('config 种子')
  })
})

test.describe('rail 历史组展开态持久（round14 4.4，agent-workbench delta）', () => {
  test('展开 → reload 仍展开；收起 → reload 仍收起（缺省收起）', async ({ page, request }) => {
    await resetState(request)
    await page.goto('/')
    const rail = new ProjectRail(page)
    await expect(rail.host).toBeVisible({ timeout: 15_000 })

    const head = rail.host.locator('[data-testid="history-group-head"]')
    const chevron = head.locator('.chevron')

    // 缺省收起（全新上下文 = 无存储记录）。
    await expect(head).toBeVisible({ timeout: 15_000 })
    await expect(head).toHaveAttribute('aria-expanded', 'false')

    // 展开：toggle 即写 localStorage。
    await chevron.click()
    await expect(head).toHaveAttribute('aria-expanded', 'true')

    // reload：展开态恢复（D-1-1 不再回弹）。
    await page.reload()
    await expect(rail.host).toBeVisible({ timeout: 15_000 })
    await expect(rail.host.locator('[data-testid="history-group-head"]')).toHaveAttribute(
      'aria-expanded',
      'true',
      { timeout: 15_000 },
    )

    // 收起：记录随 toggle 移除，reload 后回到缺省收起。
    await rail.host.locator('[data-testid="history-group-head"] .chevron').click()
    await expect(rail.host.locator('[data-testid="history-group-head"]')).toHaveAttribute(
      'aria-expanded',
      'false',
    )
    await page.reload()
    await expect(rail.host).toBeVisible({ timeout: 15_000 })
    await expect(rail.host.locator('[data-testid="history-group-head"]')).toHaveAttribute(
      'aria-expanded',
      'false',
      { timeout: 15_000 },
    )
  })
})

test.describe('创建弹窗可选预命名（round14 4.7，agent-workbench delta）', () => {
  test('带 title 创建：rail 行即显该名，免单独改名步骤', async ({ page, request }) => {
    test.setTimeout(60_000)
    await resetState(request)
    const { name: projectName } = await ensureSceneProject(request)

    await page.goto('/')
    const rail = new ProjectRail(page)
    await rail.ensureProjectExpanded(projectName)
    await rail.openNewSessionDialog(projectName)
    await rail.pickDialogAgent('claude')

    const titleInput = rail
      .newSessionDialog()
      .locator('[data-testid="dialog-title-input"] input')
    await titleInput.click()
    await titleInput.pressSequentially('gui-round14-titled')
    await rail.confirmNewSessionDialog()

    // 占位会话以该名出现在 rail（创建后立即重命名兜底链的可见半边）。
    await expect(rail.sessionItem('gui-round14-titled')).toBeVisible({ timeout: 15_000 })
  })
})
