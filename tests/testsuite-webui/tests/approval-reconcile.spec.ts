/**
 * Journey — 审批面对账纵深（fix-webui-qa-defects-round3 tasks 7.1/7.2）。
 *
 * 功能：审批卡片旅程 / 子功能：相位对账退避与挂载期拉取去重
 *
 * 7.1（相位对账退避）：推送半边缺席时（重启重拉 / 丢帧窗口），审查卡由
 * 「相位帧说 waiting + 读模型重取」补齐——但读模型拉取可能先于审批落库，
 * 合并扑空后相位值停在 waiting 不再翻转，此前对账只由相位值变化触发一次，
 * 卡片就此缺席至 reload。修复后扑空即退避重试（250ms 翻倍至 4s 封顶）。
 * 本旅程用 page.route 模拟「拉取先于落库」：泊车审批真实存在（fake-claude
 * 'perm'），但 approvals 读模型先一律应答空——waiting 下退避重取真实发生
 * （≥2 次扑空）后放行透传，下一轮重试必须免刷新免相位翻转补出卡片。
 *
 * 「相位说 waiting」的来源：dashboard 给 review-card 喂 detail 的 status_slug
 * ——本轮实施期曾发现 detail 投影漏做泊车合并（列表/相位帧/summary 在
 * review 3c 补过，detail 漏了），泊车期间 detail 说 working、把 waiting 相位
 * 遮死，7.1 的退避在默认形态下到不了位；当时旅程以 fulfill detail 的舞台
 * 绕行。实现修复落地（api.rs detail 投影补 `with_parked_approvals`，与
 * session_phase_frame 同款：remote 二选一后并入 derive）后，本旅程已拆掉
 * detail 舞台、直跑真实投影。
 *
 * 7.2（挂载期拉取去重）：挂载/换会话时 approvals GET 恰好一次（三路并发
 * 收敛为单次请求的 in-flight 共享），且卡片照常从读模型重建；pullSeq 防陈
 * 旧语义不回归（此处只断言次数与卡片出现）。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  FocusedSession,
  ProjectRail,
  ReviewCards,
  resetState,
  waitListStatus,
  waitStatus,
} from './helpers/index'

test.describe('审批卡片旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('相位对账与挂载去重', () => {
    test('waiting-phase empty pull backs off and retries until the card lands without any phase change (round3 7.1)', async ({
      page,
    }) => {
      test.setTimeout(90_000)
      const rail = new ProjectRail(page)
      const detail = new FocusedSession(page)
      const cards = new ReviewCards(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)
      // A 作无关聚焦宿主；B 的审批在页面打开前真实泊车（无任何推送消费者，
      // 也不会有 permission.requested 到达 B 的审查卡）。
      const keyA = await createSession(page.request, { prompt: 'idle-reconcile-a' })
      await waitStatus(page.request, keyA, ['done'])
      const keyB = await createSession(page.request, { prompt: 'perm' })
      // 等待走无副作用的列表轮询——detail 读取即设服务端焦点（api.rs 深链
      // 语义），页面打开前的等待若走 detail 也会平白挪动焦点指针。
      await waitListStatus(page.request, keyB, ['waiting'], 30_000)

      // 「拉取先于落库」模拟：arm 之后 approvals 读模型应答空（真实语义里
      // 的扑空窗口），翻面后恢复透传（审批此刻已落库）。
      let serveEmpty = true
      let emptyPullsForKeyB = 0
      await page.route(/\/api\/sessions\/[^/]+\/approvals$/, async (route) => {
        const url = route.request().url()
        if (serveEmpty) {
          if (url.includes(keyB)) emptyPullsForKeyB += 1
          await route.fulfill({
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify({ approvals: [] }),
          })
          return
        }
        await route.continue()
      })

      await page.goto(`/sessions/${keyA}`)
      await expect(detail.userTurn('idle-reconcile-a')).toBeVisible({ timeout: 15_000 })
      // reload 探针：补卡必须发生在同一文档里（免刷新）。
      await page.evaluate(() => {
        ;(window as unknown as { __reconcileProbeLoaded?: boolean }).__reconcileProbeLoaded = true
      })

      // 聚焦切到 B：挂载拉取扑空（arm 中），waiting 相位到达后对账扑空 →
      // 退避重试继续扑空。等 ≥2 次 B 的扑空拉取，证明「waiting 且无卡」
      // 的重试环真实在转（旧实现：相位值不再变 → 永不再拉）。
      await rail.ensureProjectExpanded(projectName)
      await rail.sessionItem('perm').click()
      await detail.expectStatus('waiting', 20_000)
      // 对账门的相位入参确实是 waiting：真实 detail 投影补上泊车合并后，
      // 泊车中的会话 detail 如实说 waiting（dashboard 原样喂给组件）。
      await expect
        .poll(
          async () =>
            page.locator('sebas-review-cards').evaluate((el) => {
              const probe = el as unknown as { sessionPhase?: string | null }
              return probe.sessionPhase ?? null
            }),
          { timeout: 10_000 },
        )
        .toBe('waiting')
      await expect
        .poll(() => emptyPullsForKeyB, { timeout: 10_000 })
        .toBeGreaterThanOrEqual(2)
      // 扑空窗口内卡片必须缺席（空读模型不渲染卡——不是 UI 撒谎）。
      await expect(cards.all()).toHaveCount(0)

      // 落库放行：下一轮退避重试（≤4s 封顶）拉到真实读模型 → 卡片出现。
      serveEmpty = false
      await expect(cards.all().first()).toBeVisible({ timeout: 10_000 })
      await expect(cards.card().locator('.head .tool')).toHaveText('Bash')
      await expect(cards.card().locator('pre.args')).toContainText('rm -rf /')
      // 卡片出现不依赖相位翻转：rail 行仍亮 waiting（决策前相位不变）。
      await detail.expectStatus('waiting', 10_000)
      // 免刷新佐证：初载探针仍在（reload 会清掉它）。
      expect(
        await page.evaluate(
          () => (window as unknown as { __reconcileProbeLoaded?: boolean }).__reconcileProbeLoaded,
        ),
      ).toBe(true)

      // 收尾：deny 让泊车回合收敛（下一用例从干净态开始）。
      await cards.deny().click()
      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })
      await waitListStatus(page.request, keyB, ['done'], 30_000)

      expect(collector.clean()).toEqual([])
    })

    test('mount and session switch pull the approvals read model exactly once (round3 7.2)', async ({
      page,
    }) => {
      test.setTimeout(90_000)
      const rail = new ProjectRail(page)
      const detail = new FocusedSession(page)
      const cards = new ReviewCards(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)
      const keyA = await createSession(page.request, { prompt: 'idle-dedup-a' })
      await waitStatus(page.request, keyA, ['done'])
      const keyB = await createSession(page.request, { prompt: 'perm' })
      await waitListStatus(page.request, keyB, ['waiting'], 30_000)

      const pullsForKeyB: string[] = []
      await page.route(/\/api\/sessions\/[^/]+\/approvals$/, async (route) => {
        if (route.request().url().includes(keyB)) pullsForKeyB.push(route.request().url())
        await route.continue()
      })

      // 换会话半边：从 A 点 rail 行切到 B——挂载（sessionKey 变更）+ waiting
      // 相位（同帧或随后到达）都只允许发出一次 GET，且卡片照常从读模型重建。
      await page.goto(`/sessions/${keyA}`)
      await expect(detail.userTurn('idle-dedup-a')).toBeVisible({ timeout: 15_000 })
      await rail.ensureProjectExpanded(projectName)
      await rail.sessionItem('perm').click()
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await page.waitForTimeout(1_500) // 对账/防抖窗口沉寂后再数——多一路就是回归
      expect(pullsForKeyB).toHaveLength(1)

      // 冷加载半边：直达 waiting 会话的深链——挂载期三路收敛为一次 GET，
      // 卡片出现（「恰好一次 GET 且卡片照常出现」）。
      pullsForKeyB.length = 0
      await page.goto(`/sessions/${keyB}`)
      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })
      await page.waitForTimeout(1_500)
      expect(pullsForKeyB).toHaveLength(1)

      // 收尾：deny 让泊车回合收敛。
      await cards.deny().click()
      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })
      await waitStatus(page.request, keyB, ['done'], 30_000)

      expect(collector.clean()).toEqual([])
    })
  })
})
