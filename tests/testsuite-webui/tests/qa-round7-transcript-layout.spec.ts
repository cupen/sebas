/**
 * Journey — fix-webui-qa-round7 1.1/1.3（agent-workbench「转录任意内容不破坏
 * 布局」的浏览器半边）。
 *
 * QA 缺陷（DEF-01）：正文折行只覆盖七种标签且用不参与 min-content 计算的
 * break-word，2100+ 字符无空格消息把转录滚动容器撑到万级 px、用户气泡被推
 * 出视口，且该会话持续受损。修复（1.2）把约束补在内容层：
 * `.body` 升级 `overflow-wrap:anywhere`、markdown `table` 块级横向滚动、
 * `.meta .author` 收缩守卫。本旅程用真实后端 + 真渲染钉住合同：
 *
 *   1. 无空格超长 token：发送 2100+ 字符连续 `x`，断言转录滚动容器
 *      （sebas-transcript-view 的 .scroll）scrollWidth 不超其视口宽度（+2px
 *      舍入余量）、用户气泡完整落在可视区内；切换主题（重渲染）与离开再
 *      回到该会话后逐项复断言，横向滚动位置不漂移（scrollLeft 恒 0）。
 *   2. 富 markdown（fake-claude 触发词 `table`）：宽 GFM 表格以块级横向滚动
 *      呈现（表格自身 scrollWidth > clientWidth 且不撑破容器）；长行代码块
 *      保持 pre 滚动语义（内容不折行、自身可横滚，anywhere 不回退）；无空
 *      格 CJK 长句折行不撑破面板（捎带回归 1.2 的 CJK 观感）。
 *
 * 断言为何抓得住旧缺陷：旧样式下 2100 字符 token 的固有宽度直接成为滚动容
 * 器的 scrollWidth（万级 px），sw-cw ≤ 2 与「气泡 bbox 在视口内」两条在首
 * 次断言即红；主题重渲染复断言挡住「初始渲染对、重渲染漂移」的回归面。
 * fake-claude 原本没有表格/代码块触发词（用户侧消息是纯 <p>，markdown 管线
 * 只服务 agent 回复），故桩新增 `table` 触发词——与 perm/drip/parallel 同机
 * 制的确定性旅程数据源。
 *
 * retries: 0：布局缺陷不得被 retry 掩盖。零固定 sleep（轮询等待回合终态）。
 */
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  FocusedSession,
  createSession,
  ensureSceneProject,
  resetState,
  waitStatus,
} from './helpers/index'

test.describe('fix-webui-qa-round7 转录布局', () => {
  test.describe.configure({ retries: 0 })

  // 固定视口：断言「scrollWidth 不超视口」需要有确定性的视口宽度。
  test.use({ viewport: { width: 1280, height: 720 } })

  let collector: ErrorCollector
  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })
  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  /** 转录滚动容器（transcript-view shadow DOM 的 .scroll）度量。 */
  function scrollMetrics(page: import('@playwright/test').Page) {
    return page.locator('sebas-transcript-view .scroll').evaluate((el) => ({
      scrollWidth: el.scrollWidth,
      clientWidth: el.clientWidth,
      scrollLeft: el.scrollLeft,
    }))
  }

  /** 布局合同：容器不横向溢出（+2px 舍入余量）、无横向滚动漂移。 */
  async function expectNoHorizontalOverflow(page: import('@playwright/test').Page) {
    await expect
      .poll(
        async () => {
          const m = await scrollMetrics(page)
          return m.scrollWidth - m.clientWidth
        },
        { timeout: 10_000 },
      )
      .toBeLessThanOrEqual(2)
    const m = await scrollMetrics(page)
    expect(m.scrollLeft, 'horizontal scroll position must not drift').toBe(0)
  }

  test('2100+ 字符无空格消息不撑破面板；主题切换与重进入复断言', async ({
    page,
    request,
  }) => {
    const detail = new FocusedSession(page)

    await resetState(request)
    await ensureSceneProject(request)
    // 0-turn 占位 + 深链聚焦（qa-round6 同款装配）：消息经真实 composer 提交。
    const key = await createSession(request, { prompt: null })
    await page.goto(`/sessions/${key}`)
    await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })

    // 2100+ 字符无空格消息（DEF-01 的字面复现输入）。
    const longToken = 'x'.repeat(2100)
    await detail.sendFollowUp(longToken)
    await expect(detail.userTurn('xxxx')).toBeVisible({ timeout: 20_000 })
    await waitStatus(request, key, ['done'])

    // 合同一：转录滚动容器不横向溢出；用户气泡完整落在视口内。
    await expectNoHorizontalOverflow(page)
    const viewport = page.viewportSize()!
    const bubbleBox = await page
      .locator('sebas-transcript-view .turn-block.is-user .msg-block')
      .boundingBox()
    expect(bubbleBox, 'user bubble must render').not.toBeNull()
    expect(bubbleBox!.x).toBeGreaterThanOrEqual(0)
    expect(bubbleBox!.x + bubbleBox!.width).toBeLessThanOrEqual(viewport.width)

    // 合同二（主题重渲染）：切深色（pre-paint 脚本 + 全量重渲染），布局不
    // 回漂——气泡仍完整可见、容器仍不横向溢出、滚动位置仍 0。
    await page.evaluate(() => localStorage.setItem('sebas:theme', 'dark'))
    await page.reload()
    await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })
    expect(
      await page.evaluate(() => document.documentElement.classList.contains('wa-dark')),
      'theme flip must actually take effect (the re-assert is vacuous otherwise)',
    ).toBe(true)
    await expect(detail.userTurn('xxxx')).toBeVisible({ timeout: 20_000 })
    await expectNoHorizontalOverflow(page)
    const darkBox = await page
      .locator('sebas-transcript-view .turn-block.is-user .msg-block')
      .boundingBox()
    expect(darkBox!.x).toBeGreaterThanOrEqual(0)
    expect(darkBox!.x + darkBox!.width).toBeLessThanOrEqual(viewport.width)

    // 切回浅色：同一合同在另一侧调色板下成立。
    await page.evaluate(() => localStorage.setItem('sebas:theme', 'light'))
    await page.reload()
    await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })
    await expect(detail.userTurn('xxxx')).toBeVisible({ timeout: 20_000 })
    await expectNoHorizontalOverflow(page)

    // 合同三（离开再回到该会话）：重进入后布局与滚动位置不漂移。
    await page.goto('/')
    await page.goto(`/sessions/${key}`)
    await expect(detail.userTurn('xxxx')).toBeVisible({ timeout: 20_000 })
    await expectNoHorizontalOverflow(page)
  })

  test('宽 GFM 表格块级横滚、pre 滚动语义不回退、CJK 长句折行不撑破', async ({
    page,
    request,
  }) => {
    const detail = new FocusedSession(page)

    await resetState(request)
    await ensureSceneProject(request)
    const key = await createSession(request, { prompt: null })
    await page.goto(`/sessions/${key}`)
    await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })

    // 触发词 `table`：agent 回一封富 markdown（160 列宽表格 + 220 字符长行
    // 代码块 + 无空格 CJK 长句），回合收敛后定稿渲染走 markdown 管线。
    await detail.sendFollowUp('table')
    const table = page.locator('sebas-transcript-view .turn-block .body table')
    await expect(table).toBeVisible({ timeout: 20_000 })
    await waitStatus(request, key, ['done'])

    // 转录容器本体不横向溢出（表格、代码块、CJK 段都在其内）。
    await expectNoHorizontalOverflow(page)

    // 表格以块级横向滚动呈现：fixture 是 160 列表格，min-content 宽度必然
    // 超出正文列——表格自身内容宽（scrollWidth）超过其盒宽（clientWidth，
    // = 正文列宽），即横向滚动条在表格自身上，而不是把父容器撑破。
    // （内容可折行的普通文字表格会被 .body 继承的 overflow-wrap:anywhere
    // 压进容器——该压缩行为作为观感发现另行上报，不在本断言面。）
    const tableMetrics = await table.evaluate((el) => ({
      scrollWidth: el.scrollWidth,
      clientWidth: el.clientWidth,
    }))
    expect(tableMetrics.scrollWidth).toBeGreaterThan(tableMetrics.clientWidth)
    expect(tableMetrics.clientWidth).toBeLessThanOrEqual(1280)

    // pre 代码块：white-space:pre 语义不回退——220 字符无空格行不折行
    // （scrollWidth > clientWidth），横向滚动由 pre 自身承担。
    const pre = page.locator('sebas-transcript-view .turn-block .body pre')
    await expect(pre).toBeVisible()
    const preMetrics = await pre.evaluate((el) => ({
      scrollWidth: el.scrollWidth,
      clientWidth: el.clientWidth,
    }))
    expect(preMetrics.scrollWidth).toBeGreaterThan(preMetrics.clientWidth)

    // CJK 长句如实上屏且在容器内（sw-cw 断言已覆盖不撑破；此处补内容存在）。
    const bodyText = await page
      .locator('sebas-transcript-view .turn-block.is-assistant .body')
      .first()
      .textContent()
    expect(bodyText).toContain('中文连续长句')
  })
})
