/**
 * Journey — fix-webui-qa-round7 3.2（webui-ws-rpc「未认证客户端不反复撞升级
 * 端点」的浏览器半边）。跑在 auth-on 形态（playwright.auth.config.ts，
 * TESTSUITE_AUTH=1 沙箱，admin/admin）。
 *
 * QA 缺陷：升级被拒（未认证 401）曾走固定 30s 慢梯反复重试——登录页停留
 * 期间 console 周期性刷 `/ws` 升级失败错误。修复（3.1）：`everOpened=false`
 * 的 close 视作疑似未认证，**不排任何重连定时器**（静默等待态），解除复用
 * 既有 `setAuthGated(false)`（登录成功）+ `reconnectNow` 路径。
 *
 * 钉住的合同：
 *   1. 登录页停留 35s：`/ws` 升级失败恰好 1 条（sharedWs 模块装载的首连，
 *      spec 明文允许）——旧实现 30s 慢梯在本窗口必然出现第 2 条，恰好吃中
 *      周期性重试；新实现静默等待永不出现。
 *   2. admin/admin 登录后 WS 建立、实时事件链路可用：UI 登录成功 → 对既有
 *      会话经 API 追加一条消息，agent 回复**不经刷新**实时上屏（WS
 *      turn.append 事件推送，拉取轮询不在断言面）。
 *
 * retries: 0：重连语义缺陷不得被 retry 掩盖。35s 观察窗是覆盖旧 30s 慢梯
 * 一个完整周期的最小窗（test 级 timeout 相应放宽）。
 */
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  FocusedSession,
  Login,
  authLogin,
  createSession,
  sendMessage,
} from './helpers/index'

test.describe('fix-webui-qa-round7 未认证 WS 重连闸', () => {
  test.describe.configure({ retries: 0 })

  let collector: ErrorCollector
  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })
  test.afterEach(() => {
    // 登录页那一条模块装载首连失败是 spec 明文允许的既有形状（ErrorCollector
    // 对 ws://127.0.0.1 的 close 噪音有窄过滤），此处只挡 pageerror。
    expect(collector.pageErrors).toEqual([])
  })

  test('登录页停留期 console 无周期性 /ws 升级失败（首连 1 条后静默）', async ({
    page,
  }) => {
    test.setTimeout(75_000)
    // 只收集 /ws 升级失败的 console 错误（既有 ErrorCollector 对本地 ws 噪音
    // 整体放行，这里需要按形状计数）。
    const wsFailures: string[] = []
    page.on('console', (msg) => {
      if (msg.type() === 'error' && /WebSocket connection to 'ws:\/\/127\.0\.0\.1:\d+\/ws'/.test(msg.text())) {
        wsFailures.push(msg.text())
      }
    })

    await page.goto('/')
    const login = new Login(page)
    await expect(login.host).toBeVisible({ timeout: 15_000 })

    // 35s 观察窗：覆盖旧实现的第一个 30s 慢梯重试点——新实现（不排定时器）
    // 在此窗口内只有模块装载的 1 条首连失败，旧实现必然出现第 2 条。
    await page.waitForTimeout(35_000)

    expect(
      wsFailures.length,
      `unauthenticated login page must not repeatedly hit the /ws upgrade (got ${wsFailures.length}): ${JSON.stringify(wsFailures)}`,
    ).toBe(1)
  })

  test('admin/admin 登录后 WS 建立，agent 回复实时上屏（无刷新）', async ({
    page,
    request,
  }) => {
    // API 侧独立登录（request fixture 不带浏览器的 cookie）。
    expect(await authLogin(request, 'admin', 'admin')).toBe(200)
    const key = await createSession(request, { prompt: 'ws round7' })
    await sendMessage(request, key, 'hello')

    // UI 侧登录进工作台。
    await page.goto('/')
    const login = new Login(page)
    await expect(login.host).toBeVisible({ timeout: 15_000 })
    await login.login('admin', 'admin')
    await expect(page.locator('sebas-dashboard')).toBeVisible({ timeout: 15_000 })

    // WS 已建立：断线驻留横幅缺席（app-shell 只在 ready 态 setWsDown——
    // 「与服务器的连接已断开…」横幅驻留即 WS 未建立的信号）。
    await expect(
      page.locator('sebas-notice-layer').getByText('与服务器的连接已断开'),
    ).toHaveCount(0)

    // 深链聚焦会话；首屏渲染走初始拉取，之后的增量只能来自 WS 推送。
    const detail = new FocusedSession(page)
    await page.goto(`/sessions/${key}`)
    await expect(detail.agentTurn('world').first()).toBeVisible({ timeout: 20_000 })
    await expect(detail.composerTextarea).toBeVisible()
    const before = (await detail.bubbles().allTextContents()).length

    // 实时事件链路：页面已就位后经 API 追加一条消息，其气泡**不经刷新**上屏
    // ——只能走 WS turn.append 推送。
    await sendMessage(request, key, 'realtime probe')
    await expect
      .poll(async () => (await detail.bubbles().allTextContents()).length, {
        timeout: 20_000,
      })
      .toBeGreaterThan(before)
    await expect(
      page.locator('sebas-notice-layer').getByText('与服务器的连接已断开'),
    ).toHaveCount(0)
  })
})
