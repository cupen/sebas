/**
 * Journey — deployment resilience of the core session channel
 * (harden-core-channel-deployment 5.4, design D6/D7).
 *
 * Runs against the DETACHED dual-process topology (playwright.detached.config.ts):
 * core + standalone webui with NO SEBAS_CORE_SECRET — auto-arm + secret-file
 * discovery. The journey stops the core mid-flight and asserts the browser
 * turns honest WITHOUT any reload: the global fatal notice appears
 * (add-webui-tiered-notices: persistent banner + workbench lock overlay —
 * browsing locks too), the composer gates submission, and a project added
 * during the outage lands in the local registry with the degradation hint;
 * after the core comes back (fresh generated key, same config) the banner
 * and the lock are gone on the WS-pushed reachability flip (no polling).
 */
import { expect, test } from '@playwright/test'
import { mkdirSync } from 'node:fs'
import path from 'node:path'
import {
  AppShell,
  ErrorCollector,
  ProjectRail,
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

test.describe('部署韧性', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('核心通道停启', () => {
    test('core 停 → fatal 横幅 + 锁定、composer 门禁、加项目降级提示；core 恢复 → 解锁 + 恢复通知', async ({
      page,
    }) => {
      const scene = detachedSceneDir()
      const shell = new AppShell(page)
      const rail = new ProjectRail(page)
      const workbench = new Workbench(page)

      await page.goto('/')
      await expect(shell.brand).toBeVisible()
      // Retry self-healing: a previous attempt (or teardown race) may have
      // left the scene's core stopped — restore the healthy baseline first,
      // so the flip below is always exercised from reachable.
      if ((await reachabilityOk(page.request)) !== true) {
        if (!isCoreAlive(scene)) {
          startCore(scene)
        }
        await waitForCoreReachability(page.request, true, 45_000)
      }
      // Self-sufficient focus: since interaction-polish the composer only
      // renders for a FOCUSED session. This journey used to lean on whatever
      // server-side focus pointer an earlier spec (approval-detached runs
      // first alphabetically) left in the persistent scene — running
      // `--case deployment` alone landed on the empty state instead. Create
      // a real session and deep-link to it so the baseline holds standalone;
      // the session survives the stop/start below via graceful state dump.
      const key = await createSession(page.request, { prompt: 'deployment-probe' })
      await waitStatus(page.request, key, ['done'], 30_000)
      await page.goto(`/sessions/${key}`)
      // 起点健康：无 fatal 横幅与锁定遮罩，composer 可提交。
      await expect(page.locator('[data-testid="core-fatal-banner"]')).toHaveCount(0)
      await expect(workbench.composerTextarea).toBeEnabled()

      // ── core 停止（SIGTERM 优雅退出：socket 移除，密钥文件保留）────────
      await stopCore(scene)

      // fatal 横幅即时出现（WS 推送驱动，无轮询节奏）：kind 分档文案 + cause
      // 原文。detached 拓扑停核是 SIGTERM 优雅退出、socket 文件随之移除，
      // 通道客户端重连呈 ENOENT → startup_failed（「核心启动失败」）；
      // 握手后的掉线才是 disconnected，旧 core 退化无 kind 时为通用文案。
      const banner = page.locator('[data-testid="core-fatal-banner"]')
      await expect(banner).toBeVisible({ timeout: 15_000 })
      await expect(
        page.locator('[data-testid="core-fatal-banner"]').locator('.banner'),
      ).toHaveAttribute('role', 'alert')
      await expect(banner).toContainText(/核心启动失败|与核心的连接已断开|核心不可达/)

      // 锁定遮罩：工作台整体 inert（交互与浏览一并锁住），遮罩卡载恢复提示。
      await expect(page.locator('[data-testid="core-lock-overlay"]')).toBeVisible()
      await expect(page.locator('[data-testid="core-lock-overlay"]')).toContainText(
        '核心恢复后将自动解锁并刷新工作台',
      )
      const inert = await page
        .locator('wa-split-panel.frame')
        .evaluate((el) => el.hasAttribute('inert'))
      expect(inert).toBe(true)

      // composer 门禁：输入与发送键随不可达禁用，就地展示 cause。
      await expect(workbench.composerTextarea).toBeDisabled({ timeout: 15_000 })
      await expect(workbench.sendButton).toBeDisabled()
      await expect(workbench.composer).toContainText('core not connected')

      // 「停核期间经 UI 加项目看降级提示」自 add-webui-tiered-notices 的
      // fatal 全锁起不可达：rail 连同加项目入口一并 inert（本文件上方已断言
      // inert），UI 路径让位锁定语义。降级注册契约由 API 级测试承载
      // （session_endpoints_test.rs projects_add_degraded_when_core_unreachable：
      // 201 + degraded.cause），需求文本不变。

      // ── core 恢复（同 config 重启：自动装配生成新钥，webui 文件发现自愈）──
      startCore(scene)
      await waitForCoreReachability(page.request, true, 45_000)

      // 无需刷新页面：恢复翻转推送到达后横幅与锁定即时消失，并弹出
      // 「核心已恢复」info toast（分级通知层的恢复语义）。
      await expect(page.locator('[data-testid="core-fatal-banner"]')).toHaveCount(0, {
        timeout: 15_000,
      })
      await expect(page.locator('[data-testid="core-lock-overlay"]')).toHaveCount(0)
      const inertAfter = await page
        .locator('wa-split-panel.frame')
        .evaluate((el) => el.hasAttribute('inert'))
      expect(inertAfter).toBe(false)
      await expect(
        page.locator('wa-toast-item').filter({ hasText: '核心已恢复' }),
      ).toBeVisible({ timeout: 15_000 })
      // composer 解禁需要聚焦会话。重启后深链会话可能已不在 summary 里
      // （恢复面与快照的时序差，convergence 清 deepLinkKey 后不回头），
      // 所以恢复后新建一个会话并深链聚焦——确定性成立，同时证明恢复后的
      // core 能正常开新会话（发真消息、回合收敛）。
      const recovered = await createSession(page.request, { prompt: 'deployment-recovered' })
      await waitStatus(page.request, recovered, ['done'], 30_000)
      await page.goto(`/sessions/${recovered}`)
      await expect(workbench.composerTextarea).toBeEnabled({ timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })
  })
})
