/**
 * Journey — 未读徽标跨层旅程（rail-declutter-unread：session-unread-badge）。
 *
 * 功能：未读徽标 / 子功能：真实回复点亮徽标、聚焦清零、锚落 localStorage。
 *
 * 此前该 capability 只散在组件单测（project-rail / unread-cursor / transcript
 * view 拿 mock 行渲染徽标）与服务端单测（count_chat_messages 口径、API 行
 * 投影），没有任何一层把「真实 agent 回复 → msg_count 经 API 可见 → 徽标在
 * 不刷新的页面上亮起 → 聚焦清零且读锚落 localStorage」这条旅程接起来——本
 * 用例就是那条接缝。
 *
 * 场景基座：fake-claude 默认 hello 场景（"hello " + "world" 相邻 delta 合并
 * 为一段）→ 每个完成回合 msg_count 恰好 +1，徽标数字确定。
 *
 * 钉住的 delta scenario：
 * - first visit shows no unread（两行 msg_count=1 而无任何锚点 → 无红点）
 * - new reply on an unfocused session（A 离焦时收到新段，徽标无需刷新亮起）
 * - focusing the session clears the badge（点击聚焦后徽标消失，anchor_count
 *   == 服务端当前 msg_count——seam 与徽标共用的同一游标）
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  listSessions,
  ProjectRail,
  resetState,
  sendMessage,
  waitStatus,
} from './helpers/index'

test.describe('未读徽标', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('a reply to an unfocused session lights the badge; focusing clears it and parks the anchor', async ({
    page,
  }) => {
    test.setTimeout(90_000)
    const rail = new ProjectRail(page)

    await resetState(page.request)
    // rail-declutter-unread：会话须绑定项目才出现在 rail；行名 = 首 prompt。
    const { id: projectId, name: projectName } = await ensureSceneProject(page.request)
    const tagA = `badge-a-${Date.now()}`
    const tagB = `badge-b-${Date.now()}`
    const keyA = await createSession(page.request, { prompt: tagA, projectId })
    const keyB = await createSession(page.request, { prompt: tagB, projectId })
    await waitStatus(page.request, keyA, ['done'])
    await waitStatus(page.request, keyB, ['done'])

    // 服务端投影（跨层证据）：一个完成回合 = 1 个可见回复段。
    const rowCount = () =>
      listSessions(page.request).then((rows) => rows.find((r) => r.encoded_key === keyA)?.msg_count)
    await expect.poll(rowCount).toBe(1)

    await page.goto('/')
    await expect(rail.host).toBeVisible()
    await rail.expandProject(projectName)

    // 首访无锚点 = 全部已读：两行都带着 msg_count=1，却都不冒红点。
    await expect(rail.host.locator('[data-testid="session-unread"]')).toHaveCount(0)

    // 聚焦 A（写锚 anchor=1）再聚焦 B——A 变成离焦会话，页面不刷新。
    await rail.sessionItem(tagA).click()
    await expect(rail.sessionItem(tagA)).toHaveAttribute('aria-current', 'true', {
      timeout: 10_000,
    })
    await rail.sessionItem(tagB).click()
    await expect(rail.sessionItem(tagB)).toHaveAttribute('aria-current', 'true', {
      timeout: 10_000,
    })
    await expect(rail.sessionItem(tagA)).not.toHaveAttribute('aria-current', 'true')

    // A 收到新回复（API 投影先钉住 +1），rail 徽标随后亮起——无需任何 reload。
    // 实施期发现（见 COVERAGE.md）：rail 行名跟随**最新** prompt（后端
    // emit_turn_card 每轮 drop+re-seed CardState，prompt_preview 投影随之
    // 变化），与 delta「named by the first prompt」字面不符——所以此段不按
    // 旧标签找行，直接断言徽标（此刻 rail 里唯一可能未读的就是 A：B 已读）。
    await sendMessage(page.request, keyA, 'again')
    await waitStatus(page.request, keyA, ['done'])
    await expect.poll(rowCount).toBe(2)
    const badgeRow = rail.host
      .locator('li.session-item', { has: page.locator('[data-testid="session-unread"]') })
    const badge = badgeRow.locator('[data-testid="session-unread"]')
    await expect(badgeRow).toHaveCount(1, { timeout: 15_000 })
    await expect(badge).toHaveText('1')
    await expect(badge).toHaveAttribute('title', '1 条未读回复')

    // 聚焦 A（点徽标所在行）：徽标消失，读锚推进到服务端当前 msg_count
    // （=2，seam 与徽标共用）。
    await badgeRow.click()
    await expect(badgeRow).toHaveCount(0, { timeout: 10_000 })
    const anchor = await page.evaluate((k) => {
      const raw = localStorage.getItem(`sebas:seen:${k}`)
      return raw ? (JSON.parse(raw) as { seen_ts: number; anchor_count: number | null }) : null
    }, keyA)
    expect(anchor?.anchor_count).toBe(2)
    expect(anchor?.seen_ts).toBeGreaterThan(0)

    expect(collector.clean()).toEqual([])
  })
})
