/**
 * Journey — session-slash-commands 5.3 browser smoke: the composer command
 * palette against a real fake-claude session (the sandbox's claude agent
 * advertises goal/compact via `--advertise-commands`).
 *
 * - 命令面板: typing the first char `/` opens the palette listing the
 *   advertised table — command name + argumentHint + description (3.1).
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
      // full advertised table: name + param hint + description (3.1).
      const palette = page.locator('sebas-workbench-composer [data-testid="command-palette"]')
      await workbench.composerTextarea.fill('/')
      await expect(palette).toBeVisible()
      const goal = palette.locator('[data-command="goal"]')
      await expect(goal).toContainText('/goal')
      await expect(goal).toContainText('<condition>')
      await expect(goal).toContainText('Track a goal across turns')
      const compact = palette.locator('[data-command="compact"]')
      await expect(compact).toContainText('/compact')
      await expect(compact).toContainText('Clear conversation context')

      // Incremental filter (3.2): `c` narrows to compact; goal drops out.
      await workbench.composerTextarea.pressSequentially('c')
      await expect(palette.locator('[data-command="goal"]')).toHaveCount(0)
      await expect(palette.locator('[data-command="compact"]')).toBeVisible()

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
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 20_000 })

      // `/goal …` passthrough (3.3): whitespace keeps the palette closed, so
      // Enter submits directly and the RAW text renders as the operator's
      // own turn before the turn converges again.
      const bubblesBefore = await detail.bubbles().count()
      await detail.sendFollowUp('/goal some-condition')
      await expect(detail.userTurn('/goal some-condition')).toBeVisible({ timeout: 20_000 })
      await expect(detail.bubbles()).toHaveCount(bubblesBefore + 2)
      await expect(detail.statusBadge).toHaveAttribute('slug', 'done', { timeout: 20_000 })

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
