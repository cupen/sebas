/**
 * Journey — session-slash-commands 5.3 browser smoke: the composer command
 * palette against a real fake-claude session (the sandbox's claude agent
 * advertises goal/compact via `--advertise-commands`).
 *
 * - 命令面板: typing the first char `/` opens the palette listing the
 *   advertised table — one compact row per command: command name +
 *   argumentHint (3.1). The description is NOT rendered inline: it appears
 *   in the detail bubble anchored to the highlighted (or hovered) row
 *   (workbench-composer-input-polish 3.1/3.2).
 * - 增量过滤: typing narrows the candidates; a prefix with no match keeps
 *   the panel hidden (3.2).
 * - 两段式补全: picking a completion inserts `name + space`, keeps focus in
 *   the textarea, closes the palette; the next Enter submits (3.1) — both
 *   the click path (compact) and the direct typed path (goal).
 * - 透传与分流: `/goal …` is passed through verbatim (own turn bubble, turn
 *   converges to Done, 3.3). `/compact` is a sebas dispatch built-in
 *   (Command::Compact): the composer lets it through (no「不支持」notice),
 *   the engine forwards `/compact` to the agent and NO user-turn bubble
 *   renders (the compact flow carries no prompt transcript entry) — the
 *   assertion is acceptance + the agent turn converging.
 * - 拦截: a slash command outside (advertised ∪ {compact}) is blocked with
 *   the inline「不支持」notice and nothing is submitted (4.1).
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  FocusedSession,
  resetState,
  waitStatus,
  Workbench,
} from './helpers/index'

test.describe('slash 命令面板（session-slash-commands）', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('面板、补全与透传', () => {
    test('`/` opens the advertised palette, completion submits, goal passes through', async ({
      page,
    }) => {
      test.setTimeout(120_000)
      const detail = new FocusedSession(page)
      const workbench = new Workbench(page)

      await resetState(page.request)
      // First turn spawns the agent child — the initialize handshake
      // advertises the command table, materialized into the session detail
      // the composer reads. Wait for convergence before opening the panel.
      const key = await createSession(page.request, { prompt: 'hello' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.sessionHead).toBeVisible()

      // `/` (command-name segment, no whitespace) opens the palette with the
      // full advertised table — one compact row per command: name + param
      // hint only (workbench-composer-input-polish 3.1); the description
      // moved into the detail bubble (3.2), rendered for the keyboard
      // highlight (the palette opens with the first option highlighted; the
      // hover path shares the same rendering and is unit-covered — the
      // vertical workbench overlay intercepts pointer events here, see the
      // finding below).
      const palette = page.locator('sebas-workbench-composer [data-testid="command-palette"]')
      const bubble = page.locator('sebas-workbench-composer [data-testid="command-bubble"]')
      await workbench.composerTextarea.fill('/')
      await expect(palette).toBeVisible()
      const goal = palette.locator('[data-command="goal"]')
      await expect(goal).toContainText('/goal')
      await expect(goal).toContainText('<condition>')
      // Row collapse (3.1): the description is no longer inline in the row.
      await expect(goal).not.toContainText('Track a goal across turns')
      // Bubble (3.2): the highlighted row's description renders as sanitized
      // markdown inside the bounded bubble anchored to that row.
      await expect(bubble).toBeVisible()
      await expect(bubble).toContainText('Track a goal across turns')
      const compact = palette.locator('[data-command="compact"]')
      await expect(compact).toContainText('/compact')
      await expect(compact).not.toContainText('Clear conversation context')

      // Incremental filter (3.2): `c` narrows to compact; goal drops out and
      // the bubble follows the (reset) highlight onto compact's description.
      await workbench.composerTextarea.pressSequentially('c')
      await expect(palette.locator('[data-command="goal"]')).toHaveCount(0)
      await expect(palette.locator('[data-command="compact"]')).toBeVisible()
      await expect(bubble).toContainText('Clear conversation context')
      await expect(bubble).not.toContainText('Track a goal across turns')

      // Two-stage completion via KEYBOARD (3.1's primary path): Tab inserts
      // `compact ` (name + space), closes the palette, focus stays in the
      // textarea for the arguments.
      // （Finding: the palette items are NOT mouse-clickable in the vertical
      // workbench layout — the transcript's wa-split-panel pane visually
      // overlays the popup and intercepts pointer events. Keyboard works;
      // reported, not fixed here.）
      await expect(compact).toHaveAttribute('aria-selected', 'true')
      await workbench.composerTextarea.press('Tab')
      await expect(palette).toBeHidden()
      // Palette closed → the detail bubble collapses with it (3.2 collapse
      // timing: Esc / completion / surface absence all close the palette).
      await expect(bubble).toHaveCount(0)
      await expect(workbench.composerTextarea).toHaveValue('/compact ')

      // Enter submits. `/compact` is the dispatch built-in: accepted (no
      // 「不支持」notice, input cleared) and forwarded to the agent. The
      // compact flow writes no user prompt entry, so its reply MERGES into
      // the trailing agent bubble instead of opening a new turn block —
      // assert the reply text landed in the transcript.
      await workbench.composerTextarea.press('Enter')
      const notice = page.locator('sebas-workbench-composer [data-testid="slash-unsupported"]')
      await expect(notice).toHaveCount(0)
      await expect(workbench.composerTextarea).toHaveValue('')
      const compactReply = /hello world[\s\S]*hello world/
      await expect
        .poll(
          async () =>
            (await detail.bubbles().allTextContents()).some((t) => compactReply.test(t)),
          { timeout: 20_000 },
        )
        .toBe(true)
      await detail.expectStatus('done', 20000)
      // 相位断言会展开项目行（= 同时选中项目，工作台随 rail-select 重绘）——
      // 这一瞬气泡数可能读成 0；基线要等它落回再取。
      await expect
        .poll(async () => detail.bubbles().count(), { timeout: 10_000, intervals: [250] })
        .toBeGreaterThan(0)

      // `/goal …` passthrough (3.3): whitespace keeps the palette closed, so
      // Enter submits directly and the RAW text renders as the operator's
      // own turn before the turn converges again.
      // 基线必须等 compact 段落渲染收敛后再取：compact 回复与尾随 agent 气泡
      // 的合并是到达序敏感的渲染判定，全量负载下可能晚于 status=done 落定，
      // 迟到的 +1 会污染「+2」断言（表现为偶发 expected+2/received+3）。
      await expect
        .poll(
          async () => {
            const c1 = await detail.bubbles().count()
            await page.waitForTimeout(400)
            return c1 === (await detail.bubbles().count())
          },
          { timeout: 15_000, intervals: [500] },
        )
        .toBe(true)
      const bubblesBefore = await detail.bubbles().count()
      await detail.sendFollowUp('/goal some-condition')
      await expect(detail.userTurn('/goal some-condition')).toBeVisible({ timeout: 20_000 })
      // 回复气泡落盘耗时随负载波动——轮询到「+2」而不是一次 toHaveCount。
      await expect
        .poll(async () => detail.bubbles().count(), { timeout: 20_000, intervals: [250] })
        .toBe(bubblesBefore + 2)
      await detail.expectStatus('done', 20000)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('未支持命令拦截', () => {
    test('a command outside the advertised table is blocked with an inline notice', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const key = await createSession(page.request, { prompt: 'hello' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.sessionHead).toBeVisible()

      // Non-empty command surface + unknown command name → blocked (4.1):
      // inline warning names the offender, nothing reaches the transcript.
      const turnsBefore = await detail.bubbles().count()
      await detail.sendFollowUp('/nosuchcmd')
      const notice = page.locator('sebas-workbench-composer [data-testid="slash-unsupported"]')
      await expect(notice).toBeVisible()
      await expect(notice).toContainText('/nosuchcmd')
      await expect(detail.bubbles()).toHaveCount(turnsBefore)

      expect(collector.clean()).toEqual([])
    })
  })
})
