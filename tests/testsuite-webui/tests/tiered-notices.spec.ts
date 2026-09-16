/**
 * Journey — 分级通知层：fatal 锁定与恢复（add-webui-tiered-notices 5.2）。
 *
 * 功能：分级通知层 / 子功能：fatal 锁定下的工作台不可交互与恢复解锁
 *
 * Runs against the DETACHED dual-process topology (playwright.detached.config.ts):
 * core + standalone webui with NO SEBAS_CORE_SECRET. The journey stops the
 * core mid-flight and asserts the tiered-notice layer's fatal semantics over
 * the old "browsing stays available" behaviour: the fatal banner (kind-aware
 * headline + verbatim cause) appears with the lock overlay, the whole
 * workbench frame goes inert (rail + main — interactions AND browsing lock),
 * and after the core returns everything unlocks and a 「核心已恢复」 info
 * toast pops — all without any page reload (WS-pushed reachability, no
 * polling).
 *
 * 边界（如实上报）：「未豁免调用失败 → warn toast」本期无 UI 生产者——
 * 既有调用点全部有内联呈现、集中在 client.ts 豁免名单（spec 场景
 * 「API 操作失败自动弹 warn」由 client.test.ts 的拦截器单测 + 未来调用点
 * 承接）；「/ws 断线 → 持续 warn 横幅」的宿主是 webui 进程而非 core——
 * 停 core 不会断 /ws，旅程无法在不杀 webui 的前提下制造，单测已覆盖。
 */
import { expect, test } from '@playwright/test'
import {
  AppShell,
  ErrorCollector,
  Workbench,
  createSession,
  detachedSceneDir,
  isCoreAlive,
  reachabilityOk,
  startCore,
  stopCore,
  waitForCoreReachability,
  waitStatus,
} from './helpers/index'

test.describe('分级通知层', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('fatal 锁定与恢复', () => {
    test('core 停 → fatal 横幅 + 锁定遮罩 + 工作台 inert；core 恢复 → 解锁 + 「核心已恢复」', async ({
      page,
    }) => {
      // 只有 detached 拓扑（9897）能安全停核：共享单进程套件（9899）的
      // testIgnore 无法由本侧收紧（配置在别的 change 维护），这里按 baseURL
      // 自卫跳过，防误入共享沙箱杀核。
      const use = (test.info().config as { use?: { baseURL?: string } }).use
      const baseURL = String(use?.baseURL ?? '')
      test.skip(
        !baseURL.endsWith(':9897'),
        'fatal-lock journey runs only under playwright.detached.config.ts (port 9897)',
      )

      const scene = detachedSceneDir()
      const shell = new AppShell(page)
      const workbench = new Workbench(page)

      await page.goto('/')
      await expect(shell.brand).toBeVisible()
      // 自愈基线（对齐 deployment 旅程）：先前尝试可能在核心停着时收场。
      if ((await reachabilityOk(page.request)) !== true) {
        if (!isCoreAlive(scene)) {
          startCore(scene)
        }
        await waitForCoreReachability(page.request, true, 45_000)
      }
      // 起点要有聚焦会话（composer 只在聚焦态渲染）。
      const key = await createSession(page.request, { prompt: 'tiered-notices-probe' })
      await waitStatus(page.request, key, ['done'], 30_000)
      await page.goto(`/sessions/${key}`)
      // 基线健康：无 fatal 横幅、无锁定遮罩、composer 可交互。
      await expect(page.locator('[data-testid="core-fatal-banner"]')).toHaveCount(0)
      await expect(page.locator('[data-testid="core-lock-overlay"]')).toHaveCount(0)
      await expect(workbench.composerTextarea).toBeEnabled()

      // ── core 停止：翻转推送驱动 fatal（无需刷新、无轮询等待）────────────
      await stopCore(scene)

      const banner = page.locator('[data-testid="core-fatal-banner"]')
      await expect(banner).toBeVisible({ timeout: 15_000 })
      // kind 分档（通道断开 = disconnected 档；无 kind 的旧核退化为通用文案）
      // + cause 原文随行（内文由 banner 的 cause 小字承载，断言不钉死具体值）。
      await expect(banner).toContainText(/与核心的连接已断开|核心不可达/)
      // 锁定：遮罩 + 原因卡 + 恢复提示在场。
      const overlay = page.locator('[data-testid="core-lock-overlay"]')
      await expect(overlay).toBeVisible()
      await expect(overlay).toContainText('会话与操作已锁定')
      await expect(overlay).toContainText('核心恢复后将自动解锁并刷新工作台')
      // 工作台整体 inert（rail + main 共同祖先）：交互与浏览一并锁住。
      const inert = await page
        .locator('wa-split-panel.frame')
        .evaluate((el) => el.hasAttribute('inert'))
      expect(inert).toBe(true)
      // 锁定面证据：composer 不可交互（inert 的语义在 Chromium 命中即禁）。
      await expect(workbench.composerTextarea).toBeDisabled()

      // ── core 恢复：解锁 + 「核心已恢复」info toast（无需刷新页面）───────
      startCore(scene)
      await waitForCoreReachability(page.request, true, 45_000)

      await expect(banner).toHaveCount(0, { timeout: 15_000 })
      await expect(overlay).toHaveCount(0)
      const inertAfter = await page
        .locator('wa-split-panel.frame')
        .evaluate((el) => el.hasAttribute('inert'))
      expect(inertAfter).toBe(false)
      await expect(
        page.locator('wa-toast-item').filter({ hasText: '核心已恢复' }),
      ).toBeVisible({ timeout: 15_000 })

      // 解锁后的工作台真正可用：新会话可建、composer 可提交（恢复面收敛）。
      const recovered = await createSession(page.request, { prompt: 'tiered-notices-recovered' })
      await waitStatus(page.request, recovered, ['done'], 30_000)
      await page.goto(`/sessions/${recovered}`)
      await expect(workbench.composerTextarea).toBeEnabled({ timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })
  })
})
