/**
 * Journey 3.2 — session core round-trip (spec: 首回合往返 + 重载恢复).
 *
 * 功能：agent 对话覆盖 / 子功能：首回合往返与重载恢复
 *
 * workbench-interaction-polish D2 旅程：项目行「+」→ 创建对话框（agent
 * 预选）→ 确认创建 0-turn 占位（服务端 set_focus，会话就地激活）→
 * composer 发送第一条消息（spawn 子进程）→ the SPA STAYS on the workbench
 * and the focused session's conversation renders in place — the operator's
 * submission as its own turn bubble, fake-claude's "hello world" reply as
 * one agent bubble → status converges to Done → after a full page reload
 * the conversation and status come back from the server-side persisted
 * state (nothing lost).
 */
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  ensureSceneProject,
  ProjectRail,
  resetState,
  FocusedSession,
  Workbench,
} from './helpers/index'

test.describe('agent 对话覆盖', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('首回合往返与重载恢复', () => {
    test('composer submit → reply → done → reload restores', async ({ page }) => {
      const workbench = new Workbench(page)
      const rail = new ProjectRail(page)
      const detail = new FocusedSession(page)

      // The creation dialog is the only creation entry — it needs a
      // focused-free workbench: close everything any earlier journey left.
      await resetState(page.request)

      // 创建必须显式选项目（rail-declutter-unread D6 收口为对话框唯一入口）
      // ——先注册 scene 项目，项目行「+」打开对话框。
      const { name: projectName } = await ensureSceneProject(page.request)

      await page.goto('/')
      await expect(workbench.noFocusHint).toBeVisible()
      await rail.openNewSessionDialog(projectName)

      // Confirming creates AND activates the 0-turn placeholder (server-side
      // set_focus) — the dialog closes and the composer enters follow mode.
      // The modal close animation briefly holds the top layer: wait it out
      // before touching the composer (typing during the animation is lost).
      await rail.confirmNewSessionDialog()
      await expect(rail.newSessionDialog()).toBeHidden({ timeout: 10_000 })
      // wa-dialog 的关闭动画后还有一个异步的焦点归还簿记（restore 到触发
      // 钮）；不等它，紧跟其后的 fill/键入会被抢走的焦点吃掉。给一拍让
      // 焦点簿记落定——人类从对话框挪到输入框本就更慢。
      await page.waitForTimeout(400)
      await expect(detail.sessionHead).toBeVisible({ timeout: 15_000 })
      await expect(workbench.composerTextarea).toBeVisible()
      await expect(workbench.submitControl).toHaveAttribute('data-state', 'disabled')

      // First message into the placeholder spawns the agent child — the
      // spawn + turn-start gap is seconds-scale, so wait generously.
      await workbench.sendPrompt('hello')

      // Creation no longer navigates (workbench-conversation-view 3.x: the
      // workbench IS the conversation surface) — the SPA stays on `/` and
      // the new session's conversation appears in place, submissions included.
      await expect(detail.userTurn('hello')).toBeVisible({ timeout: 20_000 })

      // fake-claude answers "hello " + "world": ONE agent turn bubble whose
      // text carries both chunks in arrival order.
      await expect(detail.agentTurn('world').first()).toBeVisible({
        timeout: 15_000,
      })
      const bubbleTexts = await detail.bubbles().allTextContents()
      const helloIdx = bubbleTexts.findIndex((t) => t.includes('hello'))
      const worldIdx = bubbleTexts.findIndex((t) => t.includes('world'))
      expect(helloIdx).toBeGreaterThanOrEqual(0)
      expect(worldIdx).toBeGreaterThanOrEqual(helloIdx)

      // Turn converges to Done (badge slug flips on the session head).
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 15_000 })

      // Reload: focus pointer + conversation + status recover from persisted
      // state — the workbench still renders the same focused session.
      await page.reload()
      await expect(detail.sessionHead).toBeVisible()
      await expect(detail.userTurn('hello')).toBeVisible()
      await expect(detail.bubbles().filter({ hasText: 'hello' }).first()).toBeVisible()
      await expect(detail.bubbles().filter({ hasText: 'world' }).first()).toBeVisible()
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done')

      expect(collector.clean()).toEqual([])
    })
  })
})
