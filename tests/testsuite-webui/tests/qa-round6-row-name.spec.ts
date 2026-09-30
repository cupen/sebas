/**
 * Journey — fix-webui-qa-round6 2.2（project-session-actions「未发消息的占位
 * 会话行名可读」浏览器半边）。
 *
 * QA 缺陷：服务端命名链删除 user_prompt 回退后，零消息占位会话的 rail 行
 * 曾经落到 UUID 截断（session_id_short）——单测用 mock 行数据探不到真实
 * 投影链。本旅程用**真实后端**钉全链：
 *   1. 创建对话框确认 → 0-turn 占位入册：rail 行名 = 「未命名会话」
 *      （行文本与 hover title 都是可读占位，绝无短 id 截片）；
 *      API 半边：该行 prompt_preview 为空（服务端只认首条锚定值）。
 *   2. 首条消息落地：行名**原地**切到首条 prompt 预览（session.updated
 *      live 链，无刷新）——API 半边 prompt_preview 变为 'hello'。
 *
 * retries: 0：实现缺陷不得被 retry 掩盖。零固定 sleep。
 */
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  FocusedSession,
  ProjectRail,
  ensureSceneProject,
  listSessions,
  resetState,
  waitStatus,
} from './helpers/index'

test.describe('fix-webui-qa-round6 占位会话行名', () => {
  test.describe.configure({ retries: 0 })

  let collector: ErrorCollector
  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })
  test.afterEach(() => {
    expect(collector.pageErrors).toEqual([])
    expect(collector.consoleErrors).toEqual([])
  })

  test('占位行名可读（未命名会话），首条消息落地原地切到首条 prompt 预览', async ({
    page,
    request,
  }) => {
    const rail = new ProjectRail(page)
    const detail = new FocusedSession(page)
    await resetState(request)
    const { name: projectName } = await ensureSceneProject(request)
    await page.goto('/')
    await rail.ensureProjectExpanded(projectName)

    // 经创建对话框建 0-turn 占位（spec 场景的字面入口：creation dialog、
    // 尚无任何消息）。缺省预选 agent 即可，不关心模式。
    await rail.openNewSessionDialog(projectName)
    await rail.confirmNewSessionDialog()
    await expect(rail.newSessionDialog()).toBeHidden()

    // 占位会话已入册：fresh scene 里唯一 prompt_preview 为空的行就是它
    // （API 半边——服务端命名链无 user_prompt 回退，零消息 = 无预览）。
    let key: string | null = null
    await expect
      .poll(async () => {
        const row = (await listSessions(request)).find((r) => r.prompt_preview == null)
        key = row?.encoded_key ?? null
        return key
      })
      .not.toBeNull()

    // rail 行名 = 「未命名会话」：行文本与 hover title 都是可读占位——
    // 短 id 截片（旧行为）不再是任何命名来源。
    const placeholderRow = rail.sessionItem('未命名会话')
    await expect(placeholderRow).toBeVisible()
    await expect(placeholderRow.locator('.session-name')).toHaveText('未命名会话')
    await expect(placeholderRow).toHaveAttribute('title', '未命名会话')

    // 首条消息：深链聚焦占位会话 → composer 发第一条。
    await page.goto(`/sessions/${key!}`)
    await expect(detail.sessionHead).toBeVisible({ timeout: 15_000 })
    await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })
    await detail.sendFollowUp('hello')
    await waitStatus(request, key!, ['done'])

    // API 半边：首条锚定值落库。
    await expect
      .poll(async () => {
        const row = (await listSessions(request)).find((r) => r.encoded_key === key)
        return row?.prompt_preview ?? null
      })
      .toBe('hello')

    // UI 半边（同一页面、无刷新）：行名原地从「未命名会话」切到首条 prompt
    // 预览——session.updated live 链，占位词退场。
    await expect(rail.sessionItem('未命名会话')).toHaveCount(0, { timeout: 15_000 })
    const namedRow = rail.sessionItem('hello')
    await expect(namedRow).toBeVisible()
    await expect(namedRow.locator('.session-name')).toHaveText('hello')

    expect(collector.clean()).toEqual([])
  })
})
