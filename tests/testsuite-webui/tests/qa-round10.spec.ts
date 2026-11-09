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
 *  3. 「Rail history badge = archived total」（fix-webui-qa-round14 4.5 翻新
 *     口径）：agent-workbench 主 spec「The History group SHALL show the total
 *     count of archived sessions」明文——徽章 = 归档总数（与归档面板内容
 *     一致），建会话/关闭**不再**跟随 live 会话统计（round10 的
 *     total_sessions 口径与面板内容不一致，QA-1 UX 缺口 3），归档才涨。
 *
 * 数据源：fake-claude 触发词（`table` 富 markdown / `drip` 流式）——与
 * qa-round7-transcript-layout、conversation-streaming 同机制，零装配改动。
 */
import { expect, test, type Page } from '@playwright/test'
import {
  archiveSession,
  pickSelectOption,
  createSession,
  ensureSceneProject,
  ErrorCollector,
  FocusedSession,
  getSession,
  ProjectRail,
  resetState,
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
    return page.getByTestId('react-transcript').evaluate((el) => {
      const scroll = el.parentElement ?? el
      return scroll.scrollWidth - scroll.clientWidth
    })
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
    const table = page.locator('[data-testid="react-transcript"] [data-unit-key] article table')
    await expect(table).toBeVisible({ timeout: 20_000 })
    await waitStatus(request, key, ['done'])

    // 第二回合在宽表在场时直播：`drip`（3 段正文，段间 400ms，~1s 直播窗）。
    await detail.sendFollowUp('drip')
    const liveTurn = page
      .locator('[data-testid="react-transcript"] [data-unit-key]:has-text("agent")', { hasText: 'drip0' })
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

    // 结算态复测二（「inside its own region」）：宽表在自己的滚动区内横滚。
    // React 蓝图把滚动区放在表格外层的 .sebas-table-scroll 包裹容器（表格
    // 自身 max-width:100%），故横滚承诺断在包裹层：内容宽超过包裹层盒宽，
    // 且横滚不落在转录容器上（上方 horizontalOverflow 已钉）。
    const tableScroller = page.locator(
      '[data-testid="react-transcript"] [data-unit-key] article .sebas-table-scroll',
    )
    await expect(tableScroller).toBeVisible()
    const tableMetrics = await tableScroller.evaluate((el) => ({
      scrollWidth: el.scrollWidth,
      clientWidth: el.clientWidth,
    }))
    expect(tableMetrics.scrollWidth).toBeGreaterThan(tableMetrics.clientWidth)
    expect(tableMetrics.clientWidth).toBeLessThanOrEqual(1280)

    // 结算态复测三（「visible scrollbar affordance」）：滚动区带可见滚动条
    // ——Firefox 通道（computed scrollbar-width:thin）+ Chromium 通道
    // （全局 ::-webkit-scrollbar 规则，10px thumb）。
    const computed = await tableScroller.evaluate((el) => {
      const pre = el.closest('article')?.querySelector('pre')
      return {
        table: getComputedStyle(el).scrollbarWidth,
        pre: pre ? getComputedStyle(pre).scrollbarWidth : null,
      }
    })
    // 可见滚动条承诺落在 computed style（React CSS Modules 无 shadow 样式表）：
    // 表格与 pre 的滚动区都须声明 thin；未声明即实现缺口，保留断言之。
    expect(computed.table, 'table scroll region must ship a visible scrollbar').toBe('thin')
    expect(computed.pre, 'pre scroll region must ship a visible scrollbar').toBe('thin')
  })
})

test.describe('rail 历史徽章（fix-webui-qa-round14 4.5：徽章 = 归档总数）', () => {
  test.describe.configure({ retries: 1 })

  let collector: ErrorCollector
  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })
  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  test('历史组头计数 = 归档总数：建会话不动它、归档即涨（免 reload 联动）', async ({
    page,
    request,
  }) => {
    const rail = new ProjectRail(page)
    await resetState(request)
    await ensureSceneProject(request)

    await page.goto('/')
    await expect(rail.host).toBeVisible({ timeout: 15_000 })
    const badge = rail.historyToggle
    await expect(badge).toBeVisible()

    // 归档总数真值（GET /api/archive 的 archived_sessions——面板列的就是它）。
    const archivedOf = async (): Promise<number> => {
      const d = (await (await request.get('/api/archive')).json()) as {
        archived_sessions: unknown[]
      }
      return d.archived_sessions.length
    }

    // 同源：徽章文本 === 归档总数（沙箱内既有归档不管多少，动态对齐）。
    const baseline = await archivedOf()
    await expect(badge).toHaveAccessibleName(new RegExp(`历史\\s*${baseline}`), { timeout: 15_000 })

    // 建会话：徽章**不**跟随 live 会话表（round14 4.5 的行为翻转点——
    // round10 旧断言在此必然红：徽章曾随 total_sessions +1）。
    await createSession(request, { prompt: null })
    await expect(badge).toHaveAccessibleName(new RegExp(`历史\\s*${baseline}`), { timeout: 15_000 })

    // 归档：徽章跟随归档组 +1（session.removed → 刷新通道，免 reload），
    // 且与归档面板真值收敛一致。
    const key = await createSession(request, { prompt: null })
    await archiveSession(request, key)
    await expect(badge).toHaveAccessibleName(new RegExp(`历史\\s*${baseline + 1}`), { timeout: 15_000 })
    await expect
      .poll(async () => Number((await badge.textContent())?.match(/\d+/)?.[0]), { timeout: 15_000 })
      .toBe(await archivedOf())
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
    const chip = page
      .getByTestId('composer')
      .getByRole('combobox', { name: '会话模型' })
    await expect(chip).toBeVisible({ timeout: 15_000 })
    await pickSelectOption(page, chip, 'ok-model')
    await expect.poll(currentModel, { timeout: 15_000, intervals: [250] }).toBe('ok-model')
    // AntD 6 的选中标签是 .ant-select-content 的文本（combobox input 恒空）。
    const chipText = chip.locator('xpath=..')
    await expect(chipText, 'chip follows the server truth after the accepted switch').toContainText(
      'ok-model',
      { timeout: 15_000 },
    )

    // 再选 bad-model：桩拒绝 → 非终态 Error（含稳定标记「模型未变」）落转录
    // 类型化错误条目；current_model 不变；芯片回退会话有效模型（无乐观写）。
    await pickSelectOption(page, chip, 'bad-model')
    // 类型化拒绝点名模型且非静默成功：React 把非终态错误落成转录里的
    // 类型化错误条目（role=alert，含驱动侧「模型未变」标记），不是静默
    // 失败也不是线性成功。
    const rejection = page
      .getByTestId('workbench-session')
      .getByRole('alert')
      .filter({ hasText: '模型未变' })
    await expect(rejection).toBeVisible({ timeout: 15_000 })
    await expect(rejection).toContainText('bad-model')
    await expect.poll(currentModel, { timeout: 15_000, intervals: [250] }).toBe('ok-model')
    await expect(chipText).toContainText('ok-model', { timeout: 15_000 })
    expect(await chipText.textContent()).not.toContain('bad-model')
  })
})
