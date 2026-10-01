/**
 * Journey — core 连接状态常驻徽标（add-webui-round7-gaps 2.2，tasks.md 2.2）。
 *
 * 功能：连接徽标三态 / 对应 delta：agent-workbench「core 连接状态常驻指示」
 *
 * 拓扑决策（相对 tasks.md 2.2 原文的实现级修正）：原文说「复用既有死亡旅程
 * 装配（playwright.dead-core.config.ts + TESTSUITE_ALLOW_CORE_DEATH=1）」，
 * 但该装配是**单进程**形态——SIGKILL 掉的 core --webui 进程就是 webui 本身，
 * 服务死了就没有任何 `core.reachability` 推送可言，徽标只能停在最后已知态
 * （spec 场景「core 断连（ok = false，kind = disconnected 等）」的前提是
 * webui 还活着、能收到翻转推送）。因此徽标旅程落在 detached 双进程拓扑
 * （playwright.detached.config.ts，deployment/tiered-notices 同款装配）：
 * 停核由 webui 诚实感知并推送 ok=false，恢复由 startCore 自动翻转——翻转
 * 三态全部真实可达。单进程死亡形态下徽标与 ws-down 横幅的关系是另一个
 * 已知观察，随 review 报告上报，不在本 journey 断言。
 *
 * 断言（spec 三场景）与已知缺陷：
 * - 「断连时醒目且联动」「恢复自动翻转」两场景由翻转旅程承载（都是
 *   core.reachability 推送驱动，实测 ~0.3s / ~1s 落地）；
 * - 「健康时低调呈现」的 fresh load 半边已修：markAuthReady 就绪即补一次
 *   refreshCoreReachability（get 幂等；此前 onWsState 的 connected 分支在
 *   authState 未就绪时被吞、初始 get 永不发出，review 曾以 test.fail 钉住——
 *   passed」逼删注记）；恢复翻转后的健康呈现（同 spec 场景）在翻转旅程
 *   尾段正常断言。
 */
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  isCoreAlive,
  reachabilityOk,
  startCore,
  stopCore,
  detachedSceneDir,
  waitForCoreReachability,
} from './helpers/index'

test.describe('连接徽标三态', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('健康时低调呈现：fresh load 即 ok', async ({ page }) => {
    // 已知实现缺陷（add-webui-round7-gaps review 3c 实证）：初始
    // `core.reachability.get` 在 authState 就绪前被 onWsState 提前 return
    // 吞掉，fresh load 的徽标停在 unknown（spec「WHEN core 可达 THEN 指示
    // 显示正常态」不成立）。缺陷修复前本用例按预期失败；修复后它会
    // 实现取推荐路径 a：markAuthReady 就绪即补 get，fresh load 即 ok。
    const badge = page.locator('[data-testid="core-link"]')
    const banner = page.locator('[data-testid="core-fatal-banner"]')

    const scene = detachedSceneDir()
    // 健康基线（重试自愈：上次尝试可能把 core 停在停机态）。
    if ((await reachabilityOk(page.request)) !== true) {
      if (!isCoreAlive(scene)) {
        startCore(scene)
      }
      await waitForCoreReachability(page.request, true, 45_000)
    }

    await page.goto('/')
    await expect(page.locator('sebas-app')).toBeVisible()
    await expect(badge).toBeVisible({ timeout: 15_000 })
    await expect(badge).toHaveAttribute('data-state', 'ok', { timeout: 15_000 })
    await expect(badge).toContainText('核心已连接')
    await expect(badge).toHaveAttribute('title', '核心连接正常')
    await expect(badge).not.toContainText('不可达')
    await expect(banner).toHaveCount(0)

    expect(collector.clean()).toEqual([])
  })

  test('停核翻红且横幅一致（cause 悬停）；恢复自动翻回', async ({ page }) => {
    test.setTimeout(90_000)
    const scene = detachedSceneDir()
    const badge = page.locator('[data-testid="core-link"]')
    const banner = page.locator('[data-testid="core-fatal-banner"]')

    await page.goto('/')
    // Retry self-healing（deployment 同款）：先恢复健康基线，翻转永远从
    // 可达出发（fresh load 的徽标初态是未知态——known defect，见上例）。
    if ((await reachabilityOk(page.request)) !== true) {
      if (!isCoreAlive(scene)) {
        startCore(scene)
      }
      await waitForCoreReachability(page.request, true, 45_000)
    }
    await expect(page.locator('sebas-app')).toBeVisible()
    await expect(badge).toBeVisible({ timeout: 15_000 })

    // ── 停核：webui 活着，翻转推送驱动徽标翻红（实测 ~0.3s）────────────
    await stopCore(scene)
    await waitForCoreReachability(page.request, false, 30_000)

    await expect(badge).toHaveAttribute('data-state', 'down', { timeout: 15_000 })
    await expect(badge).toContainText('核心不可达')
    // 悬停轻交互：kind 分档文案 + cause 原文进 title（本装配 SIGTERM 停核
    // 实测 kind=disconnected、cause=connection dropped；不钉死具体档——
    // 与 banner 同一套分档词表，Never 靠 cause 字符串匹配）。
    await expect(badge).toHaveAttribute(
      'title',
      /核心不可达（核心启动失败|与核心的连接已断开|核心拒绝接入）：.+/,
    )
    // 同一事实两种强度：fatal 横幅同时在场，语义与徽标一致（两者同源
    // applyCoreReachability，绝不互相矛盾）。
    await expect(banner).toBeVisible({ timeout: 15_000 })
    await expect(banner).toContainText(/核心不可达|核心启动失败|与核心的连接已断开/)

    // ── 恢复：同 config 重启（auto-arm 密钥自愈），无需刷新自动翻回 ────
    startCore(scene)
    await waitForCoreReachability(page.request, true, 45_000)

    await expect(badge).toHaveAttribute('data-state', 'ok', { timeout: 15_000 })
    // 翻回后的健康低调呈现（spec「恢复自动翻回」+「健康时低调呈现」的
    // 推送翻转半边）：绿点常态词、无告警文案。
    await expect(badge).toContainText('核心已连接')
    await expect(badge).toHaveAttribute('title', '核心连接正常')
    await expect(badge).not.toContainText('不可达')
    await expect(banner).toHaveCount(0, { timeout: 15_000 })

    expect(collector.clean()).toEqual([])
  })
})
