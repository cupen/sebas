/**
 * Journey — 待执行堆叠区 (workbench-turn-queue 7.1–7.3, design D8).
 *
 * 功能：工作台堆叠区 / 子功能：忙中提交可见、刷新后仍在、归档点名丢弃、
 * 终结提示（未执行）。
 *
 * 场景基座：沙箱 fake-claude 带 `--slow-ms 800`（tasks.py 装配）。首 prompt
 * 用 "stream" 内容触发场景（5 帧 × 250ms 停顿 + settle）——WORKING 窗口
 * ≈2s，忙中提交确定性入队；帧间 stdin 可读，driver 的 watchdog 探测
 * （1.5s 超时）始终可被应答。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  ProjectRail,
  resetState,
  sendMessage,
  waitStatus,
} from './helpers/index'

test.describe('待执行堆叠区', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('busy-time submission rides the stack, survives refresh, and archive names the loss', async ({
    page,
  }) => {
    const rail = new ProjectRail(page)
    await resetState(page.request)

    // 首会话：流式场景让 turn 在飞一段时间。
    // rail-declutter-unread：会话要出现在 rail（供 … 菜单归档），须绑定项目。
    const { id: projectId, name: projectName } = await ensureSceneProject(page.request)
    const key = await createSession(page.request, { prompt: 'stream', projectId })
    await page.goto('/')

    // turn 已开轮：转录出现流式 chunk（WORKING 窗口内）。
    await expect(
      page.locator('sebas-transcript-view').first(),
    ).toContainText(/chunk/, { timeout: 15_000 })

    // 忙中提交 → 堆叠区可见该提交（不占 transcript，带处置文案）。回合在飞
    // 时条目点名阻塞条件与起等时刻（fix-pending-queue-liveness 3.2：栈要
    // 解释「为什么不前进」，不再只给裸位置词）。
    await sendMessage(page.request, key, 'queued while busy')
    const stack = page.locator('sebas-pending-stack [data-testid="pending-stack"]')
    await expect(stack).toBeVisible({ timeout: 5_000 })
    await expect(stack).toContainText('queued while busy')
    await expect(stack).toContainText('等待当前回合结束 · 第 1 位 · 已等')

    // 刷新后仍以服务端 payload 呈现（D8 对账语义）。
    await page.reload()
    await expect(page.locator('sebas-pending-stack [data-testid="pending-stack"]')).toContainText(
      'queued while busy',
      { timeout: 10_000 },
    )

    // 等两个回合（排队回合也会跑完）收敛到 done，再造一个确定性的
    // WORKING 窗口用于「带队列归档」段。
    await waitStatus(page.request, key, ['done'], 20_000)
    await sendMessage(page.request, key, 'stream')
    // 等 turn 真正进入 WORKING（首个内容帧），随后的提交才确定性入队——
    // 卡片尚在 SEED 时提交会开新轮而不是排队。
    await waitStatus(page.request, key, ['working'], 10_000)
    await sendMessage(page.request, key, 'drop me')

    // Rail 归档该会话（归档即关闭，4.2：唯一生命周期出口）：确认对话框
    // 点名「将丢弃 1 条」，确认后出现一次性「未执行」提示，逐条点名被
    // 丢弃的提交（7.3）。
    await rail.expandProject(projectName)
    const dialog = await rail.openArchiveDialog('stream')
    // wa-dialog host 在 top layer 读作 hidden——断言渲染出的内部元素
    // （与 settings.spec 的既有纪律一致）。
    const discardLine = dialog.locator('[data-testid="close-discards-pending"]')
    await expect(discardLine).toContainText('1', { timeout: 10_000 })
    await rail.confirmArchive()

    // 会话终结：未执行提示在焦点清空后仍然可见（提示是唯一记录）。
    const notice = page.locator('sebas-pending-stack [role="alert"]')
    await expect(notice).toBeVisible({ timeout: 10_000 })
    await expect(notice).toContainText('drop me')
    await expect(notice).toContainText('未被执行')

    expect(collector.clean()).toEqual([])
  })

  test('deterministic rejection of a removal surfaces a low-severity notice naming the entry', async ({
    page,
  }) => {
    // fix-pending-queue-liveness 3.3 拒绝反馈旅程：确定性拒绝（未知条目
    // 404，服务端 truth 里条目仍在）→ 分级通知低档就地呈现，点名条目与
    // 原因；堆叠区对账回服务端真相（条目仍在）。绝不无感。
    await resetState(page.request)
    await ensureSceneProject(page.request)
    const key = await createSession(page.request, { prompt: 'stream' })
    await page.goto('/')

    await expect(
      page.locator('sebas-transcript-view').first(),
    ).toContainText(/chunk/, { timeout: 15_000 })
    await sendMessage(page.request, key, 'reject me')
    const stack = page.locator('sebas-pending-stack [data-testid="pending-stack"]')
    await expect(stack).toBeVisible({ timeout: 5_000 })
    await expect(stack).toContainText('reject me')

    // 拦截 remove 调用并伪造类型化 404（真实现面同形：`{error: 文案}`）。
    // 其余请求照常透传——失败后的对账 GET 看到的服务端真相里条目仍在，
    // 两级判据落「确定性拒绝」半边。
    await page.route('**/pending/*/remove', (route) =>
      route.fulfill({
        status: 404,
        contentType: 'application/json',
        body: JSON.stringify({ error: '待执行提交不存在' }),
      }),
    )
    await stack.locator('.entry').first().locator('.remove').click()

    const toast = page.locator('wa-toast-item').filter({ hasText: '移除未生效' })
    await expect(toast).toBeVisible({ timeout: 5_000 })
    await expect(toast).toContainText('reject me')
    await expect(toast).toContainText('待执行提交不存在')
    // 对账：条目仍在服务端真相里 → 堆叠区不吞条目。
    await expect(stack).toContainText('reject me', { timeout: 10_000 })

    expect(collector.clean()).toEqual([])
  })
})
