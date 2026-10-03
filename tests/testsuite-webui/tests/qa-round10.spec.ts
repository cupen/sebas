/**
 * Journey — fix-webui-qa-round10 的浏览器半边（agent-workbench 增量）。
 *
 * 功能：agent 对话覆盖 / 子功能：转录直播态与超宽内容 + rail 历史徽章
 *
 * 单测已有的 DOM/CSS 合同（transcript-view.test.ts / project-rail.test.ts）
 * 之外，这里以**真实后端 + 真渲染**钉住两层只有浏览器能证明的合同：
 *
 *  1. 「Live transcript never blank-paints」+「Oversized content stays inside
 *     the transcript container」的布局半边：宽表回合（`table` 触发词，定稿
 *     markdown 管线）在场时提交流式回合（`drip`，400ms 间隔 3 段）——**会话
 *     仍 running 时**采样：转录滚动容器 scrollWidth 钉在布局宽（旧实现此处
 *     曾被超宽内容撑出 5350px 隐藏横向溢出）、直播条目有非零布局盒（不缺失、
 *     不隐藏）。绘制级「非零绘制尺寸」断言需截图人眼复核（GUI 复核单列），
 *     本旅程钉的是会被 CSS 回归先打破的 DOM/布局前提。
 *  2. 「the table scrolls horizontally inside its own region with a visible
 *     scrollbar」：结算后表格自身 scrollWidth > clientWidth（横滚在条目内层），
 *     且滚动区带可见滚动条样式（getComputedStyle 的 scrollbar-width:thin +
 *     影子样式里的 ::-webkit-scrollbar 通道）。
 *  3. 「Rail history badge reflects live session counts」：rail 的历史组头
 *     计数徽章与 GET /api/sessions 的 total_sessions（会话页统计同源）相等，
 *     且建会话/关闭后免整页 reload 跟随（WS/轮询刷新通道）。
 *
 * 数据源：fake-claude 触发词（`table` 富 markdown / `drip` 流式）——与
 * qa-round7-transcript-layout、conversation-streaming 同机制，零装配改动。
 */
import { expect, test, type Page } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  FocusedSession,
  getSession,
  ProjectRail,
  resetState,
  sessionPath,
  waitStatus,
  type StatusSlug,
} from './helpers/index'

/** 会话相位七词：非终态 = 回合仍在飞（conversation-streaming 同款口径）。 */
const RUNNING: StatusSlug[] = ['starting', 'queued', 'working', 'waiting']

test.describe('转录直播态与超宽内容（fix-webui-qa-round10 2.x）', () => {
  // 布局缺陷不得被 retry 掩盖（qa-round7 同款纪律）。
  test.describe.configure({ retries: 0 })
  // 固定视口：scrollWidth 断言要有确定性的布局宽。
  test.use({ viewport: { width: 1280, height: 720 } })

  let collector: ErrorCollector
  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })
  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  /** 转录滚动容器（shadow DOM 的 .scroll）的横向溢出量。 */
  function horizontalOverflow(page: Page): Promise<number> {
    return page.locator('sebas-transcript-view .scroll').evaluate((el) => el.scrollWidth - el.clientWidth)
  }

  test('宽表在场时流式回合：容器不撑破、直播条目非零布局盒；表格条目内横滚带可见滚动条', async ({
    page,
    request,
  }) => {
    test.setTimeout(120_000)
    const detail = new FocusedSession(page)

    await resetState(request)
    await ensureSceneProject(request)
    // 0-turn 占位 + 深链聚焦：消息经真实 composer 提交（qa-round7 同款装配）。
    const key = await createSession(request, { prompt: null })
    await page.goto(`/sessions/${key}`)
    await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })

    // 第一回合：`table` 触发词（宽 GFM 表 + 220 字符长代码行 + CJK 段）——
    // C-DEF-01 三次复现的前置（超宽条目在场）。
    await detail.sendFollowUp('table')
    const table = page.locator('sebas-transcript-view .turn-block .body table')
    await expect(table).toBeVisible({ timeout: 20_000 })
    await waitStatus(request, key, ['done'])

    // 第二回合在宽表在场时直播：`drip`（3 段正文，段间 400ms，~1s 直播窗）。
    await detail.sendFollowUp('drip')
    const liveTurn = page
      .locator('sebas-transcript-view .turn-block.is-assistant', { hasText: 'drip0' })
      .first()

    // 主断言（「never blank-paints」的 DOM/布局半边）：会话**仍 running** 时，
    // 直播条目已在 DOM 且带非零布局盒，容器横向溢出钉在舍入余量内。
    let liveOverflow: number | null = null
    let liveBox: { width: number; height: number } | null = null
    await expect
      .poll(
        async () => {
          const { detail: api } = await getSession(request, key)
          const slug = api?.status_slug
          if (slug === undefined || !RUNNING.includes(slug)) return 'terminal-or-unknown'
          const box = await liveTurn.boundingBox()
          if (!box || box.width <= 0 || box.height <= 0) return 'live-but-no-box'
          liveOverflow = await horizontalOverflow(page)
          liveBox = { width: box.width, height: box.height }
          return 'live'
        },
        { timeout: 20_000, intervals: [100] },
      )
      .toBe('live')
    expect(
      liveOverflow,
      'live transcript container must stay at its layout width (no thousands-px hidden overflow)',
    ).toBeLessThanOrEqual(2)
    expect(liveBox!.height, 'streaming entry must occupy non-zero layout space while live').toBeGreaterThan(0)

    await waitStatus(request, key, ['done'])

    // 结算态复测一：容器仍不横向溢出（宽表 + 流式回合全部定稿在场）。
    expect(await horizontalOverflow(page)).toBeLessThanOrEqual(2)

    // 结算态复测二（「inside its own region」）：宽表在条目内层横滚——表格
    // 自身内容宽超过盒宽（= 正文列宽），横滚不落在容器上。
    const tableMetrics = await table.evaluate((el) => ({
      scrollWidth: el.scrollWidth,
      clientWidth: el.clientWidth,
    }))
    expect(tableMetrics.scrollWidth).toBeGreaterThan(tableMetrics.clientWidth)
    expect(tableMetrics.clientWidth).toBeLessThanOrEqual(1280)

    // 结算态复测三（「visible scrollbar affordance」）：滚动区带可见滚动条
    // ——Firefox 通道（computed scrollbar-width:thin）+ Chromium 通道
    // （影子样式里的 ::-webkit-scrollbar 规则，8px 高、边框色阶 thumb）。
    const computed = await table.evaluate((el) => {
      const pre = el.parentElement?.querySelector('pre')
      return {
        table: getComputedStyle(el).scrollbarWidth,
        pre: pre ? getComputedStyle(pre).scrollbarWidth : null,
      }
    })
    expect(computed.table, 'table scroll region must ship a visible scrollbar').toBe('thin')
    expect(computed.pre, 'pre scroll region must ship a visible scrollbar').toBe('thin')
    const shadowStyles = await page
      .locator('sebas-transcript-view')
      .first()
      .evaluate((host) => {
        const root = (host as HTMLElement & { shadowRoot: ShadowRoot }).shadowRoot
        // Lit 走 adoptedStyleSheets（CSSStyleSheet，非 <style> 元素）——两处
        // 都收，cssText 拼出等效样式表文本。
        const parts: string[] = []
        for (const sheet of root.adoptedStyleSheets ?? []) {
          try {
            parts.push([...sheet.cssRules].map((r) => r.cssText).join('\n'))
          } catch {
            /* 不可读的样式表跳过（computed-style 断言已覆盖真渲染） */
          }
        }
        for (const s of root.querySelectorAll('style')) parts.push(s.textContent ?? '')
        return parts.join('\n')
      })
    expect(shadowStyles).toMatch(/\.turn-block \.body pre::-webkit-scrollbar,/)
    expect(shadowStyles).toMatch(/scrollbar-width:\s*thin/)
  })
})

test.describe('rail 历史徽章（fix-webui-qa-round10 5.1）', () => {
  test.describe.configure({ retries: 1 })

  let collector: ErrorCollector
  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })
  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  test('历史组头计数与会话页统计同源，建会话/关闭免 reload 联动', async ({ page, request }) => {
    const rail = new ProjectRail(page)
    await resetState(request)
    await ensureSceneProject(request)

    await page.goto('/')
    await expect(rail.host).toBeVisible({ timeout: 15_000 })
    const badge = rail.host.locator('[data-testid="history-session-count"]')
    await expect(badge).toBeVisible()

    const totalOf = async (): Promise<number> => {
      const d = (await (await request.get('/api/sessions')).json()) as { total_sessions: number }
      return d.total_sessions
    }

    // 同源：徽章文本 === GET /api/sessions 的 total_sessions（会话页统计区
    // 同一份数字；rail 首拉在飞行中，轮询到相等）。
    const baseline = await totalOf()
    await expect(badge).toHaveText(String(baseline), { timeout: 15_000 })

    // 建会话：徽章随统计 +1——无整页 reload（WS session.created / 轮询通道）。
    await createSession(request, { prompt: null })
    await expect(badge).toHaveText(String(baseline + 1), { timeout: 15_000 })

    // 关闭：徽章跟随统计回落（session.removed 通道）——仍与本源相等。
    const key = await createSession(request, { prompt: null })
    await expect(badge).toHaveText(String(baseline + 2), { timeout: 15_000 })
    const resp = await request.post(`${sessionPath(key)}/close`)
    expect(resp.ok(), `close must succeed: ${resp.status()}`).toBe(true)
    await expect
      .poll(
        async () => (await totalOf()) === Number(await badge.textContent()),
        { timeout: 15_000 },
      )
      .toBe(true)
  })
})

test.describe('composer 模型切换的类型化拒绝（fix-webui-qa-round10 4.x，acp-model-selection）', () => {
  // 拒绝投影走事件流 + 转录条目，时序不在断言步上——retry 不掩盖实现红。
  test.describe.configure({ retries: 0 })

  let collector: ErrorCollector
  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })
  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  test('composer 选 bad-model：切换真实下发、类型化拒绝上屏、芯片回退会话有效模型', async ({
    page,
    request,
  }) => {
    test.setTimeout(90_000)
    const detail = new FocusedSession(page)

    await resetState(request)
    await ensureSceneProject(request)
    // fakeacp 桩广告 bad-model/ok-model（初值 = 首位 bad-model）、对 bad-model
    // 的 set_config_option 回 RPC 错误——spec 场景「Composer picking a
    // reject-listed model surfaces the typed rejection」的 oracle 装配。
    const key = await createSession(request, { prompt: 'model-reject', agent: 'fakeacp' })
    await page.goto(`/sessions/${key}`)
    await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })
    const currentModel = async (): Promise<string | null | undefined> =>
      (await getSession(request, key)).detail?.current_model
    await expect
      .poll(currentModel, { timeout: 15_000, intervals: [250] })
      .toBe('bad-model')

    // 先切到 ok-model（真实下发：POST /api/sessions/{key}/model → 驱动
    // session/set_config_option）——「选已当前模型不构成切换」语义之外的正向面。
    const chip = page.locator('sebas-workbench-composer [data-testid="model-chip"]')
    await expect(chip).toBeVisible({ timeout: 15_000 })
    await chip.click()
    await page.locator('sebas-workbench-composer .menu-item[data-model="ok-model"]').click()
    await expect.poll(currentModel, { timeout: 15_000, intervals: [250] }).toBe('ok-model')
    await expect(chip, 'chip follows the server truth after the accepted switch').toContainText(
      'ok-model',
      { timeout: 15_000 },
    )

    // 再选 bad-model：桩拒绝 → 非终态 Error（含稳定标记「模型未变」）落转录
    // 类型化错误条目；current_model 不变；芯片回退会话有效模型（无乐观写）。
    await chip.click()
    await page.locator('sebas-workbench-composer .menu-item[data-model="bad-model"]').click()
    // 类型化拒绝点名模型且非静默成功（折叠体懒渲染——展开后轮询到上屏）。
    await detail.expectFoldedText('模型未变')
    await detail.expectFoldedText('bad-model')
    await expect.poll(currentModel, { timeout: 15_000, intervals: [250] }).toBe('ok-model')
    await expect(chip).toContainText('ok-model', { timeout: 15_000 })
    expect(await chip.textContent()).not.toContain('bad-model')
  })
})
