/**
 * Journey — error 级 toast 的 8s 自动消失（fix-webui-qa-defects-round5 4.2，
 * delta scenario「error 默认自动消失且参与挤占」的寿命半边）。
 *
 * 功能：分级通知层 / 子功能：error toast 寿命
 *
 * 缺陷：error 级通知此前驻留须手动关闭，异常态下层层驻留 toast 盖住工作台。
 * 修复：notify.ts ERROR_TOAST_DURATION_MS = 8_000（显式 duration=0 仍驻留）。
 * 浏览器生产者用真实调用点（dashboard 的归档恢复失败上报，视图面唯一的
 * error 级 notify），POST restore 被 route 拦截应答 500—— Journey 只断言
 * 通知层的寿命契约：出现、短暂驻留（2s 时仍在）、≈8s 自动消失（12s 内），
 * 全程无手动关闭。8s 真实等待：本套件无时钟 fake 惯例，WS 驱动的 Lit 应用
 * 挂 fake clock 会误伤重连/节流计时器，故用真实时钟（单用例 +9s 预算）。
 */
import { expect, test } from '@playwright/test'
import {
  archiveSession,
  createSession,
  ensureSceneProject,
  ErrorCollector,
  ProjectRail,
  resetState,
  waitStatus,
} from './helpers/index'

test.describe('分级通知层', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('error toast 寿命', () => {
    test('an error toast auto-dismisses after ~8s without manual closing', async ({
      page,
    }) => {
      test.setTimeout(60_000)
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)
      const tag = `toast-err-${Date.now()}`
      const key = await createSession(page.request, { prompt: tag })
      await waitStatus(page.request, key, ['done'])

      // 归档（API 半边）→ History 里出现该条目。
      await archiveSession(page.request, key)
      await page.goto('/')
      await expect(rail.host).toBeVisible()
      await rail.expandHistory()
      const archivedRow = rail.host
        .locator('li.session-item.archived', { hasText: tag })
        .first()
      await expect(archivedRow).toBeVisible({ timeout: 10_000 })

      // 打开恢复确认框，然后让 restore POST 诚实失败（沙箱注入 404 类型化
      // 拒绝——条目指向的会话已不在；失败面与 5xx 同一条 notify 通道）。
      await archivedRow.click()
      const archivedView = page.locator('sebas-dashboard [data-testid="archived-view"]')
      await expect(archivedView).toBeVisible({ timeout: 10_000 })
      await archivedView.locator('[data-testid="archived-restore"]').click()
      await expect(
        page.locator('sebas-dashboard [data-testid="restore-confirm"]'),
      ).toBeVisible({ timeout: 10_000 })
      await page.route(/\/api\/sessions\/[^/]+\/restore$/, (route) =>
        route.fulfill({
          status: 404,
          contentType: 'application/json',
          body: JSON.stringify({ error: '归档条目指向的会话不存在' }),
        }),
      )
      await page.locator('sebas-dashboard [data-testid="restore-confirm"]').click()

      // error toast 在场（视图显式上报的文案点名条目与失败）。
      const toast = page.locator('wa-toast-item').filter({ hasText: '恢复会话' })
      const t0 = Date.now()
      await expect(toast).toBeVisible({ timeout: 10_000 })
      await expect(toast).toContainText('失败')

      // 短暂驻留：2s 后仍在——不是瞬时即消（区别于渲染抖动）。
      await page.waitForTimeout(2_000)
      await expect(toast).toBeVisible()

      // 默认 8s 自动消失（12s 窗口覆盖沙箱抖动；驻留旧语义 = 永不消失，
      // 必红）。消失后 store 回写，同文案不被驻留面钉死。
      await expect(toast).toBeHidden({ timeout: 12_000 })
      const lifespan = Date.now() - t0
      expect(lifespan).toBeGreaterThanOrEqual(8_000)
      expect(lifespan).toBeLessThan(20_000)

      expect(collector.clean()).toEqual([])
    })
  })
})
