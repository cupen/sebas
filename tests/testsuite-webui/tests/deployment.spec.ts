/**
 * Journey — deployment resilience of the core session channel
 * (harden-core-channel-deployment 5.4, design D6/D7).
 *
 * 功能：部署韧性 / 子功能：核心停启期间的诚实外显与恢复
 *
 * Runs against the DETACHED dual-process topology (playwright.detached.config.ts):
 * core + standalone webui with NO SEBAS_CORE_SECRET — auto-arm + secret-file
 * discovery. The journey stops the core mid-flight and asserts the browser
 * turns honest WITHOUT any reload: the global "核心不可达" banner appears with
 * the reported cause, the composer gates submission, and a project added
 * during the outage lands in the local registry with the degradation hint;
 * after the core comes back (fresh generated key, same config) the banner is
 * gone on the next reachability poll.
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
    test('core 停 → 横幅含 cause、composer 门禁、加项目降级提示；core 恢复 → 横幅消失', async ({
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
      // 起点健康：无核心横幅，composer 可提交。
      await expect(page.locator('[data-testid="core-unreachable-banner"]')).toHaveCount(0)
      await expect(workbench.composerTextarea).toBeEnabled()

      // ── core 停止（SIGTERM 优雅退出：socket 移除，密钥文件保留）────────
      await stopCore(scene)

      // 全局横幅出现且携带 reachability 上报的 cause（前端 5s 轮询节奏）。
      const banner = page.locator('[data-testid="core-unreachable-banner"]')
      await expect(banner).toBeVisible({ timeout: 15_000 })
      await expect(banner).toHaveAttribute('role', 'alert')
      await expect(banner).toContainText('核心不可达：')
      await expect(banner).toContainText('会话与项目面暂不可用')

      // composer 门禁：输入与发送键随不可达禁用，就地展示 cause。
      await expect(workbench.composerTextarea).toBeDisabled({ timeout: 15_000 })
      await expect(workbench.sendButton).toBeDisabled()
      await expect(workbench.composer).toContainText('core not connected')

      // 加项目：注册成功（本地注册表降级）并就地提示，而非静默。路径每次
      // 尝试唯一：重试 attempt 里上一次已注册同一路径会 409，hint 就看不到。
      const outageDir = path.join(scene, `outage-${Date.now()}`)
      mkdirSync(outageDir, { recursive: true })
      await rail.openAddDialog()
      await rail.addProjectByPath(outageDir)
      // 先等项目落栏（= add 之后的 refresh 已完成），再要降级提示——
      // hint 在 refresh 完成后立即置位，两者同帧可达。
      const projectName = outageDir.split(/[\\/]/).filter(Boolean).pop()!
      await expect(rail.projectRow(projectName)).toBeVisible({ timeout: 15_000 })
      const hint = page.locator('[data-testid="project-degraded-hint"]')
      await expect(hint).toBeVisible({ timeout: 15_000 })
      await expect(hint).toContainText('核心不可达')
      await expect(hint).toContainText('已写入本地注册表')

      // ── core 恢复（同 config 重启：自动装配生成新钥，webui 文件发现自愈）──
      startCore(scene)
      await waitForCoreReachability(page.request, true, 45_000)

      // 无需刷新页面：横幅在下一次可达性轮询后消失。
      await expect(page.locator('[data-testid="core-unreachable-banner"]')).toHaveCount(0, {
        timeout: 15_000,
      })
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
