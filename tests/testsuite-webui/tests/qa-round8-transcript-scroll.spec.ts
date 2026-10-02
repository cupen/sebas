/**
 * 转录滚动跟随与「跳到最新」浮标的浏览器级复测（fix-webui-qa-round8 3.2）。
 *
 * 覆盖 agent-workbench「转录滚动跟随与跳到最新」的三个 scenario：
 * - 「未读缝在场时新条目到达」：开卷带未读缝的会话（定位停在中部、sticky
 *   保持脱离——round8 修复面），随后新条目到达 → 浮标出现；
 * - 「点击浮标恢复跟随」：点浮标 → 转录回底、浮标消失，此后新条目自动跟随；
 * - 「打开已全读的会话」：开卷即贴底自动跟随（round8 之前开卷定位会把
 *   sticky 无条件翻 false——自动滚动整体停摆的根因），新条目到达不出现浮标。
 *
 * 确定性基座（与 unread-seam.spec.ts 同款双会话 + 徽标重聚焦路径）：会话 A
 * 先垫四个回合保证转录溢出视口，未读内容用 `table` 触发词（fake-claude 的
 * 富 markdown 高回复，未读尾段远高于半屏）——缝居中定位后距底余量必然超
 * 过浮标阈值（NEAR_BOTTOM_PX = 80），不会踩「末回合太矮、居中落点贴底」
 * 的几何坑。聚焦敏感段遵守 README ⁵ 纪律：状态等待走 waitListStatus
 * （列表读取无焦点副作用）。
 */
import { expect, test, type Page } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  ProjectRail,
  resetState,
  sendMessage,
  waitListStatus,
} from './helpers/index'

test.describe('转录滚动跟随与跳到最新（fix-webui-qa-round8）', () => {
  // 矮视口放大「内容溢出」的确定性：~300px 滚动面 × 四个回合必溢出。
  test.use({ viewport: { width: 1180, height: 560 } })
  // retries 1：全量链尾部跑这条时，沙箱 core 已连跑 ~140 条旅程，fake-claude
  // 子进程偶发被驱动挂起探测收走（会话投影 dormant——3 次全量链 2 次，且均
  // 不在契约断言步上）；重试全新建会话，不会掩盖断言本身的红（flaky 照常
  // 在报告里可见）。
  test.describe.configure({ retries: 1 })

  let collector: ErrorCollector
  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** 转录滚动容器的距底余量（px）；> 80 = 视口下方还有内容。 */
  async function distanceFromBottom(page: Page): Promise<number> {
    return page
      .locator('sebas-transcript-view .scroll')
      .evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)
  }

  const pill = (page: Page) => page.locator('sebas-transcript-view [data-testid="jump-latest"]')

  /**
   * 垫高转录：全部垫高消息一口气发出（引擎串行排队执行），统一等 done。
   * 全量链尾部沙箱 core 已连跑上百条旅程，子进程偶发被收走（会话投影
   * dormant）——dormant 按操作者路径自愈：再发一条（Dormant resume）后重
   * 等一轮；垫高只求内容溢出，多一条不少一条都成立。
   */
  async function padSession(
    request: Parameters<typeof sendMessage>[0],
    key: string,
    rounds: string[],
  ): Promise<void> {
    for (const msg of rounds) await sendMessage(request, key, msg)
    try {
      await waitListStatus(request, key, ['done'], 45_000)
    } catch {
      await sendMessage(request, key, `pad-${Date.now()}`)
      await waitListStatus(request, key, ['done'], 45_000)
    }
  }

  test('未读缝开卷后新条目到达浮标出现；点浮标回底并恢复自动跟随', async ({ page }) => {
    test.setTimeout(120_000)
    const rail = new ProjectRail(page)
    await resetState(page.request)
    const { name: projectName } = await ensureSceneProject(page.request)
    const tagA = `seam-a-${Date.now()}`
    const tagB = `seam-b-${Date.now()}`

    // 会话 A 垫四个回合（转录溢出视口），B 用来切走制造「离焦到达」。
    const keyA = await createSession(page.request, { prompt: tagA })
    await padSession(page.request, keyA, ['round-2', 'round-3', 'round-4'])
    const keyB = await createSession(page.request, { prompt: tagB })
    await waitListStatus(page.request, keyB, ['done'])

    await page.goto('/')
    await expect(rail.host).toBeVisible()
    await rail.ensureProjectExpanded(projectName)
    // 先聚焦 A（立锚）再切到 B——A 变离焦会话。
    await rail.sessionItem(tagA).click()
    await expect(rail.sessionItem(tagA)).toHaveAttribute('aria-current', 'true', {
      timeout: 10_000,
    })
    await rail.sessionItem(tagB).click()
    await expect(rail.sessionItem(tagB)).toHaveAttribute('aria-current', 'true', {
      timeout: 10_000,
    })

    // A 在离焦状态收到高未读尾段（table 场景的富 markdown 回复）。
    await sendMessage(page.request, keyA, 'table')
    await waitListStatus(page.request, keyA, ['done'], 45_000)
    const badgeRow = rail.host.locator('li.session-item', {
      has: page.locator('[data-testid="session-unread"]'),
    })
    await expect(badgeRow).toHaveCount(1, { timeout: 15_000 })

    // 重聚焦 A：未读缝开卷——定位停在中部（seam 可见），sticky 保持脱离。
    await badgeRow.click()
    await expect(badgeRow).toHaveCount(0, { timeout: 10_000 })
    const seam = page.locator('sebas-dashboard sebas-transcript-view div.seam:not([hidden])')
    await expect(seam).toBeVisible({ timeout: 15_000 })
    // 开卷停在缝上：距底余量远超浮标阈值（未读尾段是高表格）。
    await expect
      .poll(async () => distanceFromBottom(page), { timeout: 10_000 })
      .toBeGreaterThan(80)

    // 缝在场时新条目到达：浮标出现——新内容不要求操作者手动滚动。
    await sendMessage(page.request, keyA, 'table')
    await expect(pill(page)).toBeVisible({ timeout: 20_000 })

    // 点浮标：回底 + 浮标消失 + 此后自动跟随（sticky 重新咬合）。
    await pill(page).click()
    await expect(pill(page)).toBeHidden({ timeout: 5_000 })
    await expect
      .poll(async () => distanceFromBottom(page), { timeout: 10_000 })
      .toBeLessThanOrEqual(80)

    // 恢复跟随的实证：再来的新回合把操作者带着走——落定后仍贴底，浮标
    // 不再出场。
    await sendMessage(page.request, keyA, 'round-5')
    await waitListStatus(page.request, keyA, ['done'], 45_000)
    await expect
      .poll(async () => distanceFromBottom(page), { timeout: 15_000 })
      .toBeLessThanOrEqual(80)
    await expect(pill(page)).toBeHidden()

    expect(collector.clean()).toEqual([])
  })

  test('打开已全读的会话保持贴底自动跟随，新条目不触发浮标', async ({ page }) => {
    test.setTimeout(90_000)
    await resetState(page.request)
    await ensureSceneProject(page.request)
    const key = await createSession(page.request, { prompt: 'fully-read-1' })
    // 垫高到溢出（否则「贴底」是平凡真：无滚动余量时距底恒 0）。
    await padSession(page.request, key, ['fully-read-2', 'fully-read-3', 'fully-read-4'])

    // 深链打开 = 打开一个没有未读内容的会话：开卷即贴底跟随（sticky 保持
    // true——round8 修复面），浮标不出场。
    await page.goto(`/sessions/${key}`)
    await expect
      .poll(async () => distanceFromBottom(page), { timeout: 10_000 })
      .toBeLessThanOrEqual(80)
    await expect(pill(page)).toBeHidden()

    // 新条目到达自动滚动：落定后仍贴底，浮标全程不出现。
    await sendMessage(page.request, key, 'fully-read-5')
    await waitListStatus(page.request, key, ['done'], 45_000)
    await expect
      .poll(async () => distanceFromBottom(page), { timeout: 15_000 })
      .toBeLessThanOrEqual(80)
    await expect(pill(page)).toBeHidden()

    expect(collector.clean()).toEqual([])
  })
})
