/**
 * Journey — 可拖拽分割线与浮岛布局（workbench-interaction-polish 5.1–5.3，
 * design D1/D6）。
 *
 * - 侧栏|主区水平分割：拖拽改变 rail 宽度，clamp 180–480，宽度记忆在
 *   localStorage `sebas.rail-width`，刷新恢复。
 * - 会话流|输入框垂直分割：拖拽改变 composer 高度，记忆在
 *   `sebas.composer-height`，刷新恢复。
 * - 浮岛视觉基线：nav / 舞台是圆角浮岛，分隔缝 rest 态透明、hover 亮把手
 *   （DOM/样式契约级断言，像素级视觉走沙箱截图验收）。
 * - 窄屏退化：<640px 分割线不可拖。
 *
 * 选择器纪律：document.querySelector 不穿 shadow DOM，一律用 Playwright
 * locator（自动穿透 open shadow root）；rail 分隔缝与 composer 分隔缝都
 * 匹配 `.divider`（前者包住后者），rail 缝取 `.first()`。
 */
import { expect, test, type Locator, type Page } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  resetState,
} from './helpers/index'

test.describe('工作台布局分割', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  const railSplit = (page: Page): Locator =>
    page.locator('sebas-app wa-split-panel.frame .divider').first()
  const composerSplit = (page: Page): Locator => page.locator('wa-split-panel.vsplit .divider')

  /** Panel 已完成首次布局测量（宽度/高度 > 0）。 */
  async function measured(locator: Locator): Promise<number> {
    return locator.evaluate((el) => {
      const box = el.getBoundingClientRect()
      return Math.max(box.width, box.height)
    })
  }

  test('dragging the rail/main divider resizes the rail and persists across reload', async ({
    page,
  }) => {
    await resetState(page.request)
    await ensureSceneProject(page.request)
    await page.goto('/')
    const frame = page.locator('sebas-app wa-split-panel.frame')
    await expect(frame).toBeVisible()
    await expect.poll(async () => measured(frame), { timeout: 10_000 }).toBeGreaterThan(0)

    const railBox = () => page.locator('sebas-app nav').boundingBox()
    const before = (await railBox())?.width ?? 0

    const divider = railSplit(page)
    const box = (await divider.boundingBox())!
    const startX = box.x + box.width / 2
    const startY = box.y + box.height / 2
    await page.mouse.move(startX, startY)
    await page.mouse.down()
    await page.mouse.move(startX + 120, startY, { steps: 8 })
    await page.mouse.up()

    const after = (await railBox())?.width ?? 0
    expect(after).toBeGreaterThan(before)
    expect(after).toBeLessThanOrEqual(480 + 40) // 480 clamp + nav margin

    // 宽度写进 localStorage（约定键）。
    const stored = await page.evaluate(() => localStorage.getItem('sebas.rail-width'))
    expect(stored).not.toBeNull()
    expect(Number(stored)).toBeGreaterThanOrEqual(180)
    expect(Number(stored)).toBeLessThanOrEqual(480)

    // 刷新恢复：rail 宽度回到记忆值（±布局噪声）。
    await page.reload()
    await expect(frame).toBeVisible()
    await expect.poll(async () => measured(frame), { timeout: 10_000 }).toBeGreaterThan(0)
    const restored = (await railBox())?.width ?? 0
    expect(Math.abs(restored - after)).toBeLessThan(24)

    expect(collector.clean()).toEqual([])
  })

  test('dragging the stage/composer divider resizes the composer and persists', async ({
    page,
  }) => {
    await resetState(page.request)
    const { id: projectId } = await ensureSceneProject(page.request)
    // 一个会话聚焦（composer 进入跟随态、vsplit 在位；turn 很快收敛）。
    await createSession(page.request, { prompt: 'layout', projectId })
    await page.goto('/')
    const vsplit = page.locator('wa-split-panel.vsplit')
    await expect(vsplit).toBeVisible()
    await expect.poll(async () => measured(vsplit), { timeout: 10_000 }).toBeGreaterThan(0)

    const composerBox = () => page.locator('sebas-workbench-composer .composer').boundingBox()
    const heightBefore = (await composerBox())?.height ?? 0
    expect(heightBefore).toBeGreaterThan(0)

    const divider = composerSplit(page)
    const box = (await divider.boundingBox())!
    const startX = box.x + box.width / 2
    const startY = box.y + box.height / 2
    await page.mouse.move(startX, startY)
    await page.mouse.down()
    await page.mouse.move(startX, startY - 100, { steps: 8 })
    await page.mouse.up()

    const heightAfter = (await composerBox())?.height ?? 0
    expect(heightAfter).toBeGreaterThan(heightBefore)

    const stored = await page.evaluate(() => localStorage.getItem('sebas.composer-height'))
    expect(stored).not.toBeNull()
    expect(Number(stored)).toBeGreaterThanOrEqual(120)

    // 刷新恢复（读回的值被尊重）。
    await page.reload()
    await expect(vsplit).toBeVisible()
    await expect.poll(async () => measured(vsplit), { timeout: 10_000 }).toBeGreaterThan(0)
    await expect
      .poll(async () =>
        Number(await page.evaluate(() => localStorage.getItem('sebas.composer-height'))),
      )
      .toBe(Number(stored))

    expect(collector.clean()).toEqual([])
  })

  test('regions read as floating islands; divider shows a handle on hover', async ({ page }) => {
    await resetState(page.request)
    await ensureSceneProject(page.request)
    await page.goto('/')
    await expect(page.locator('sebas-app wa-split-panel.frame')).toBeVisible()

    // 浮岛：nav 圆角卡片；canvas 与浮岛底色是两个色阶。
    const navRadius = await page
      .locator('sebas-app nav')
      .evaluate((el) => getComputedStyle(el).borderRadius)
    expect(parseFloat(navRadius)).toBeGreaterThan(0)
    const hostBg = await page
      .locator('sebas-app')
      .evaluate((el) => getComputedStyle(el).backgroundColor)
    const navBg = await page
      .locator('sebas-app nav')
      .evaluate((el) => getComputedStyle(el).backgroundColor)
    expect(hostBg).not.toBe(navBg)

    // 分隔缝 rest 透明、hover 亮起（150ms 过渡，等稳再读）。
    const divider = railSplit(page)
    const restBg = await divider.evaluate((el) => getComputedStyle(el).backgroundColor)
    await divider.hover()
    await page.waitForTimeout(300)
    const hoverBg = await divider.evaluate((el) => getComputedStyle(el).backgroundColor)
    expect(restBg).toMatch(/rgba\(0, 0, 0, 0\)|transparent/)
    expect(hoverBg).not.toBe(restBg)

    // 舞台浮岛存在（项目头部 + 对话流合为一张圆角卡）。
    await expect(page.locator('sebas-dashboard .stage-island')).toBeVisible()

    expect(collector.clean()).toEqual([])
  })

  test('narrow viewport degrades: dividers disabled and layout stacks', async ({ page }) => {
    await resetState(page.request)
    await ensureSceneProject(page.request)
    await page.setViewportSize({ width: 375, height: 720 })
    await page.goto('/')
    await expect(page.locator('sebas-app wa-split-panel.frame')).toBeVisible()

    // 分割面板禁拖（disabled 属性）。
    await expect(page.locator('sebas-app wa-split-panel.frame')).toHaveAttribute('disabled', '')
    // 分隔缝隐藏。
    await expect(railSplit(page)).toBeHidden()

    expect(collector.clean()).toEqual([])
  })
})
