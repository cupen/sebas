/**
 * Journey — core 连接状态常驻指示（add-webui-round7-gaps 2.2）。
 *
 * 拓扑：detached 双进程（playwright.detached.config.ts，React 入口端口
 * 5288）——停核由 webui 诚实感知并推送 ok=false，恢复由 startCore 自动
 * 翻转。React 顶栏用 role=status（已连接 / 连接已断开）承载连接态，fatal
 * 可达性则落到 role=alertdialog「核心不可达」锁定面；迁移后的断言以这两
 * 个公共语义为准（Lit 的 core-link 三态徽标 testid 不存在——已在报告里
 * 列为实现差异）。
 */
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  detachedSceneDir,
  isCoreAlive,
  reachabilityOk,
  startCore,
  stopCore,
  waitForCoreReachability,
} from './helpers/index'
import { ReactShell } from './helpers/pages/react'

test.describe('连接状态指示', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('健康时低调呈现：fresh load 即已连接', async ({ page }) => {
    const shell = new ReactShell(page)
    const fatal = page.getByRole('alertdialog', { name: '核心不可达' })

    const scene = detachedSceneDir()
    // 健康基线（重试自愈：上次尝试可能把 core 停在停机态）。
    if ((await reachabilityOk(page.request)) !== true) {
      if (!isCoreAlive(scene)) {
        startCore(scene)
      }
      await waitForCoreReachability(page.request, true, 45_000)
    }

    await page.goto('/')
    await expect(shell.app).toBeVisible()
    await expect(shell.connection).toContainText('已连接', { timeout: 15_000 })
    await expect(fatal).toHaveCount(0)

    expect(collector.clean()).toEqual([])
  })

  test('停核出现 fatal 锁定（cause 分档）；恢复自动解锁', async ({ page }) => {
    test.setTimeout(90_000)
    const scene = detachedSceneDir()
    const shell = new ReactShell(page)
    const fatal = page.getByRole('alertdialog', { name: '核心不可达' })

    await page.goto('/')
    // Retry self-healing（deployment 同款）：翻转永远从可达出发。
    if ((await reachabilityOk(page.request)) !== true) {
      if (!isCoreAlive(scene)) {
        startCore(scene)
      }
      await waitForCoreReachability(page.request, true, 45_000)
    }
    await expect(shell.app).toBeVisible()
    await expect(fatal).toHaveCount(0, { timeout: 15_000 })

    // ── 停核：webui 活着，翻转推送驱动锁定面 ─────────────────────────────
    await stopCore(scene)
    await waitForCoreReachability(page.request, false, 30_000)

    await expect(fatal).toBeVisible({ timeout: 15_000 })
    await expect(fatal).toContainText(/核心不可达|核心启动失败|核心连接已断开/)

    // ── 恢复：同 config 重启（auto-arm 密钥自愈），无需刷新自动解锁 ────
    startCore(scene)
    await waitForCoreReachability(page.request, true, 45_000)

    await expect(fatal).toHaveCount(0, { timeout: 15_000 })
    await expect(shell.connection).toContainText('已连接')

    expect(collector.clean()).toEqual([])
  })
})
