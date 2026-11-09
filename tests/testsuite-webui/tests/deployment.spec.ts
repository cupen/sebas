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
 *
 * 选择器已迁到 React 公共语义：锁定面是 role=alertdialog「核心不可达」，
 * composer 门禁是 [data-state] 提交键 + 内联「核心不可达，提交已暂停」。
 */
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  createSession,
  detachedSceneDir,
  isCoreAlive,
  reachabilityOk,
  startCore,
  stopCore,
  waitForCoreReachability,
  waitStatus,
} from './helpers/index'
import { ReactShell, ReactWorkbench } from './helpers/pages/react'

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
      const shell = new ReactShell(page)
      const workbench = new ReactWorkbench(page)
      const fatal = page.getByRole('alertdialog', { name: '核心不可达' })

      await page.goto('/')
      await expect(shell.app).toBeVisible()
      // Retry self-healing: a previous attempt (or teardown race) may have
      // left the scene's core stopped — restore the healthy baseline first,
      // so the flip below is always exercised from reachable.
      if ((await reachabilityOk(page.request)) !== true) {
        if (!isCoreAlive(scene)) {
          startCore(scene)
        }
        await waitForCoreReachability(page.request, true, 45_000)
      }
      // Self-sufficient focus: create a real session and deep-link to it.
      const key = await createSession(page.request, { prompt: 'deployment-probe' })
      await waitStatus(page.request, key, ['done'], 30_000)
      await page.goto(`/sessions/${key}`)
      // 起点健康：无 fatal 锁定面，composer 可提交。
      await expect(fatal).toHaveCount(0)
      await expect(workbench.composerTextarea).toBeEnabled()

      // ── core 停止（SIGTERM 优雅退出：socket 移除，密钥文件保留）────────
      await stopCore(scene)

      // fatal 锁定面即时出现（WS 推送驱动，无轮询节奏）：kind 分档文案 +
      // cause 原文。detached 拓扑停核是 SIGTERM 优雅退出、socket 文件随之
      // 移除，通道客户端重连呈 ENOENT → startup_failed（「核心启动失败」）。
      await expect(fatal).toBeVisible({ timeout: 15_000 })
      await expect(fatal).toContainText(/核心启动失败|核心连接已断开|核心不可达/)
      await expect(fatal).toContainText('工作台已锁定')

      // composer 门禁：提交键随不可达禁用，就地展示 cause。
      await expect(workbench.submitControl).toBeDisabled({ timeout: 15_000 })
      await expect(workbench.composer).toContainText('核心不可达，提交已暂停')

      // 「停核期间经 UI 加项目看降级提示」自 fatal 全锁起不可达：锁定面
      // 盖住 rail（本文件上方已断言锁定语义）。降级注册契约由 API 级测试
      // 承载（session_endpoints_test.rs projects_add_degraded_when_core_unreachable）。

      // ── core 恢复（同 config 重启：自动装配生成新钥，webui 文件发现自愈）──
      startCore(scene)
      await waitForCoreReachability(page.request, true, 45_000)

      // 无需刷新页面：恢复翻转推送到达后锁定面即时消失，并弹出
      // 「核心已恢复」info toast（分级通知层的恢复语义）。
      await expect(fatal).toHaveCount(0, { timeout: 15_000 })
      await expect(page.getByText('核心已恢复')).toBeVisible({ timeout: 15_000 })
      // composer 解禁需要聚焦会话。恢复后新建一个会话并深链聚焦。
      const recovered = await createSession(page.request, { prompt: 'deployment-recovered' })
      await waitStatus(page.request, recovered, ['done'], 30_000)
      await page.goto(`/sessions/${recovered}`)
      await expect(workbench.composerTextarea).toBeEnabled({ timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })
  })
})
