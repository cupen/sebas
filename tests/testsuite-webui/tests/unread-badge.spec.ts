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
 *
 * 第二条用例（fix-webui-qa-defects-round3 delta「Focused session arrivals
 * never badge」）：聚焦会话的流式到达经 transcript 贴底推进共享读锚——行不
 * 点灯不是因为「聚焦抑制」遮住旧账，而是锚真的推进了（localStorage 佐证）；
 * 重复点击同会话行（no-op switch）把锚再推进到当前 msg_count，徽章保持清零。
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
  waitListStatus,
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
    await rail.ensureProjectExpanded(projectName)

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
    // 焦点安全等待：detail 读取即设焦点（api.rs 深链语义）——聚焦 B 后再
    // 轮询 A 的详情会把焦点偷回 A，「聚焦到达不点灯」反把徽标吞掉；改走
    // 无焦点副作用的列表轮询。
    await waitListStatus(page.request, keyA, ['done'])
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
      return raw ? (JSON.parse(raw) as { anchor_count?: unknown }) : null
    }, keyA)
    expect(anchor?.anchor_count).toBe(2)
    // （2.3，D3）单字段段锚：存储只有 anchor_count，时间戳锚 seen_ts 已退役
    // （含 seen_ts 的旧 JSON 读为无锚 = fully read，下次写入被纯段锚覆写）。
    expect(Object.keys(anchor ?? {}).sort()).toEqual(['anchor_count'])

    expect(collector.clean()).toEqual([])
  })

  test('focused arrivals never badge; repeated same-session clicks keep it cleared', async ({
    page,
  }) => {
    // round3 delta 两个 scenario：Streaming arrival into the focused session
    // does not badge + Repeated focus keeps the badge cleared。
    test.setTimeout(90_000)
    const rail = new ProjectRail(page)

    await resetState(page.request)
    const { id: projectId, name: projectName } = await ensureSceneProject(page.request)
    const tagA = `focused-a-${Date.now()}`
    const tagB = `focused-b-${Date.now()}`
    const keyA = await createSession(page.request, { prompt: tagA, projectId })
    const keyB = await createSession(page.request, { prompt: tagB, projectId })
    await waitStatus(page.request, keyA, ['done'])
    await waitStatus(page.request, keyB, ['done'])

    await page.goto('/')
    await expect(rail.host).toBeVisible()
    await rail.ensureProjectExpanded(projectName)

    // 聚焦 A，然后 A 自己收到流式回复（API 注入）——transcript 贴底跟读要
    // 推进共享读锚，行绝不点灯。（等待走列表轮询——detail 读取即设焦点，
    // 这里要保持「A 聚焦」不被测试自己的探针偷走。）
    await rail.sessionItem(tagA).click()
    await expect(rail.sessionItem(tagA)).toHaveAttribute('aria-current', 'true', {
      timeout: 10_000,
    })
    await sendMessage(page.request, keyA, 'arrive while focused')
    await waitListStatus(page.request, keyA, ['done'])
    const rowCount = () =>
      listSessions(page.request).then((rows) => rows.find((r) => r.encoded_key === keyA)?.msg_count)
    await expect.poll(rowCount).toBe(2)
    // 徽章缺席要覆盖到达后的完整窗口（含流式帧与对账渲染），不是快照一瞥。
    await expect(rail.host.locator('[data-testid="session-unread"]')).toHaveCount(0, {
      timeout: 10_000,
    })
    // 不是「聚焦抑制」遮旧账：锚真的被 transcript 推进到当前 msg_count=2。
    const anchorA = await page.evaluate((k) => {
      const raw = localStorage.getItem(`sebas:seen:${k}`)
      return raw ? (JSON.parse(raw) as { anchor_count?: unknown }) : null
    }, keyA)
    expect(anchorA?.anchor_count).toBe(2)

    // 非聚焦的 B 收到回复 → 徽标亮起（对照半边，聚焦不抑制离焦到达）。
    await sendMessage(page.request, keyB, 'arrive while unfocused')
    await waitListStatus(page.request, keyB, ['done'])
    const badgeRow = rail.host
      .locator('li.session-item', { has: page.locator('[data-testid="session-unread"]') })
    await expect(badgeRow).toHaveCount(1, { timeout: 15_000 })
    await expect(badgeRow).toContainText(tagB)

    // 点击聚焦 B：徽标随锚推进清零；重复点击同一行（no-op switch）不再有
    // 锚可推——徽章必须保持清零（delta「Repeated focus keeps the badge
    // cleared」，此前同会话 no-op 不触发清零逻辑的回归钉）。
    await badgeRow.click()
    await expect(rail.sessionItem(tagB)).toHaveAttribute('aria-current', 'true', {
      timeout: 10_000,
    })
    await expect(rail.host.locator('[data-testid="session-unread"]')).toHaveCount(0, {
      timeout: 10_000,
    })
    await rail.sessionItem(tagB).click()
    await expect(rail.host.locator('[data-testid="session-unread"]')).toHaveCount(0, {
      timeout: 10_000,
    })
    const anchorB = await page.evaluate((k) => {
      const raw = localStorage.getItem(`sebas:seen:${k}`)
      return raw ? (JSON.parse(raw) as { anchor_count?: unknown }) : null
    }, keyB)
    expect(anchorB?.anchor_count).toBe(2)

    expect(collector.clean()).toEqual([])
  })
})
