import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  ProjectRail,
  resetState,
  sendMessage,
  waitListStatus,
  waitStatus,
} from './helpers/index'

// fix-webui-qa-round2 2.1（D-B12+M-B216）：未读分界线（unseen-turn seam）的
// 呈现与清账。review 问题 3 的回归锁：rail 聚焦主路径上，重聚焦必现分界线、
// 已读清账、重进不重现。

test.describe('未读分界线', () => {
  test.describe.configure({ retries: 0 })
  let collector: ErrorCollector
  test.beforeEach(({ page }) => { collector = new ErrorCollector(page) })

  test('refocus after unfocused completion shows the seam; reading clears it for good', async ({ page }) => {
    test.setTimeout(120_000)
    const rail = new ProjectRail(page)
    await resetState(page.request)
    const { name: projectName } = await ensureSceneProject(page.request)
    const tagA = `seam-a-${Date.now()}`
    const tagB = `seam-b-${Date.now()}`
    const keyA = await createSession(page.request, { prompt: tagA })
    await createSession(page.request, { prompt: tagB })
    await waitStatus(page.request, keyA, ['done'])
    // 垫高会话 A（fix-webui-qa-round8 5.1 的开卷结算：未读边界若全部落在
    // 可视高度内——滚动余量 0——开卷即「全部已见」，缝不开也不该开）。缝的
    // 呈现前提是未读尾段溢出视口，与 transcript-scroll 旅程同一几何前提。
    for (const extra of ['round-2', 'round-3', 'round-4']) {
      await sendMessage(page.request, keyA, extra)
      await waitStatus(page.request, keyA, ['done'])
    }

    await page.goto('/')
    await expect(rail.host).toBeVisible()
    await rail.ensureProjectExpanded(projectName)
    // 先聚焦 A（立锚）再切到 B——A 变离焦会话。
    await rail.sessionItem(tagA).click()
    await expect(rail.sessionItem(tagA)).toHaveAttribute('aria-current', 'true', { timeout: 10_000 })
    await rail.sessionItem(tagB).click()
    await expect(rail.sessionItem(tagB)).toHaveAttribute('aria-current', 'true', { timeout: 10_000 })

    // A 在离焦状态收到新回复：徽标亮起。
    await sendMessage(page.request, keyA, 'drip')
    await waitListStatus(page.request, keyA, ['done'])
    const badgeRow = rail.host.locator('li.session-item', { has: page.locator('[data-testid="session-unread"]') })
    await expect(badgeRow).toHaveCount(1, { timeout: 15_000 })

    // 重聚焦 A：分界线必现（review 问题 3 的主路径回归——此前被 loadFocused
    // 的二次臂覆盖吃掉）。
    await badgeRow.click()
    await expect(badgeRow).toHaveCount(0, { timeout: 10_000 })
    const seam = page.locator('sebas-dashboard sebas-transcript-view div.seam:not([hidden])')
    await expect(seam).toBeVisible({ timeout: 15_000 })
    expect(await seam.getAttribute('data-count')).toBe('1')

    // 已读清账：滚读到底（mark-all-seen 出口或贴底推进）后离开再回——不再重现。
    await seam.locator('wa-button, button').first().click()
    await expect(page.locator('sebas-dashboard sebas-transcript-view div.seam:not([hidden])')).toHaveCount(0, { timeout: 10_000 })
    await rail.sessionItem(tagB).click()
    await expect(rail.sessionItem(tagB)).toHaveAttribute('aria-current', 'true', { timeout: 10_000 })
    await rail.sessionItem(tagA).click()
    await expect(rail.sessionItem(tagA)).toHaveAttribute('aria-current', 'true', { timeout: 10_000 })
    await expect(page.locator('sebas-dashboard sebas-transcript-view div.seam:not([hidden])')).toHaveCount(0)

    expect(collector.clean()).toEqual([])
  })
})
