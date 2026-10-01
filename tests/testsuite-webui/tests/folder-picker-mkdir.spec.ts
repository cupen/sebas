/**
 * Journey — 目录选择器「新建文件夹」（add-webui-round7-gaps 3.2/3.3，tasks.md
 * 3.2 + 3.3 合并收口）。
 *
 * 功能：目录选择器建目录 / 对应 delta：webui/projects「目录选择器可新建子目录」
 *
 * 被测入口是全仓唯一复用点：project-rail 的「添加项目」弹窗（代码事实：
 * new-session-dialog 不使用 folder-picker——tasks 3.3 所写「两处复用点」
 * 实际只有这一处，随本 journey 收口）。旅程三段：
 * 1. 边界内合法名创建成功：树内即时可见（局部刷新，无手工刷新）、可继续
 *    进入（新目录成为下一步创建的父目录）、folder-selected 照发（路径输入
 *    框回填）并可用于项目注册（添加项目 → rail 落行，tasks 3.3 回归）。
 * 2. 非法名类型化拒绝：`..` / 含分隔符的名字被内联中文原因拒绝，无请求
 *    之外的副作用（服务端目录清单不变）。
 * 3. 越界父目录被类型化拒绝：UI 路径**构造不出**越界父目录（树本身被
 *    browse-dirs 的 workspace root 边界圈死——这正是 fail-closed 的意义），
 *    这一半由 API oracle 承载：直接 POST /api/fs/mkdir 越界父 → 400 +
 *    中文原因 + 文件系统零副作用（服务端执法的浏览器旁证，与 server.rs
 *    路由单测同形）。
 */
import fs from 'node:fs'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import {
  ErrorCollector,
  listProjects,
  ProjectRail,
  removeProject,
  resetState,
  sceneDir,
} from './helpers/index'

test.describe('目录选择器建目录', () => {
  let collector: ErrorCollector
  let rail: ProjectRail

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
    rail = new ProjectRail(page)
  })

  /** 打开添加项目弹窗并返回 folder-picker 内的定位器束。 */
  async function openPicker(page: import('@playwright/test').Page) {
    await page.goto('/')
    await expect(rail.host).toBeVisible()
    await rail.openAddDialog()
    const dialog = rail.addDialog()
    const picker = dialog.locator('sebas-folder-picker')
    await expect(picker.locator('[data-testid="mkdir-toggle"]')).toBeVisible({
      timeout: 10_000,
    })
    return { dialog, picker }
  }

  /** 打开内联命名行并提交一个名字（click 路径，不经键盘）。 */
  async function submitMkdir(picker: import('@playwright/test').Locator, name: string) {
    await picker.locator('[data-testid="mkdir-toggle"]').click()
    const input = picker.locator('[data-testid="mkdir-name"]')
    await expect(input).toBeVisible()
    await input.fill(name)
    await picker.locator('[data-testid="mkdir-confirm"]').click()
  }

  test('边界内建目录成功：树内即时可见、可进入、可用于项目注册', async ({ page }) => {
    test.setTimeout(60_000)
    const scene = sceneDir()
    const t = Date.now()
    const dirA = `mk-outer-${t}`
    const dirB = `mk-inner-${t}`
    const pathA = path.join(scene, dirA)
    const pathB = path.join(pathA, dirB)

    await resetState(page.request)
    try {
      const { dialog, picker } = await openPicker(page)

      // 根 = workspace root（沙箱场景目录）。
      await expect(picker.locator('.root-path')).toContainText(scene, { timeout: 10_000 })

      // ── 合法名：创建成功 ────────────────────────────────────────────
      await submitMkdir(picker, dirA)
      // 无内联错误；树中即时出现（「无需手工刷新」的 UI 半边——refreshNode
      // 走局部重载，根目录 = 重载整树）。
      const itemA = picker.locator('wa-tree-item').filter({ hasText: dirA })
      await expect(itemA).toBeVisible({ timeout: 10_000 })
      // folder-selected 照发：路径输入框回填新目录（选中即选中）。
      const pathInput = dialog.locator('wa-input[label="项目路径"] input')
      await expect
        .poll(async () => (await pathInput.inputValue()).replace(/\\/g, '/'), {
          timeout: 10_000,
          intervals: [200],
        })
        .toBe(`${scene}/${dirA}`.replace(/\\/g, '/'))
      // 服务端真相：目录真实存在且 browse-dirs 立即可见（服务端半边）。
      expect(fs.existsSync(pathA)).toBe(true)

      // ── 可进入：新目录作为下一步创建的父目录 ──────────────────────
      // mkdirParent = 当前选中节点（dirA）——在它下面再建一层，证明树内
      // 可以进入新目录并继续浏览/创建（单层 × N 的连续形态，与
      // mkdir_in_nested_existing_parent_round_trips 单测互为镜像）。
      await submitMkdir(picker, dirB)
      const itemB = itemA.locator('wa-tree-item[slot="children"]').filter({ hasText: dirB })
      await expect(itemB).toBeVisible({ timeout: 10_000 })
      await expect
        .poll(async () => (await pathInput.inputValue()).replace(/\\/g, '/'), {
          timeout: 10_000,
          intervals: [200],
        })
        .toBe(`${scene}/${dirA}/${dirB}`.replace(/\\/g, '/'))
      expect(fs.existsSync(pathB)).toBe(true)

      // ── 可用于项目注册（tasks 3.3 回归）：提交弹窗 → rail 落行 ─────
      await dialog.locator('wa-button').filter({ hasText: '添加项目' }).click()
      await expect
        .poll(async () => (await listProjects(page.request)).some((p) => p.path === pathB), {
          timeout: 10_000,
          intervals: [200],
        })
        .toBe(true)
      await expect(rail.projectRow(dirB)).toBeVisible({ timeout: 10_000 })

      expect(collector.clean()).toEqual([])
    } finally {
      // Cleanup：注销 + 删除本 journey 创建的目录（注册失败也不留痕）。
      const projects = await listProjects(page.request)
      for (const p of projects.filter((x) => x.path === pathB || x.path === pathA)) {
        await removeProject(page.request, p.id)
      }
      fs.rmSync(pathA, { recursive: true, force: true })
    }
  })

  test('非法名类型化拒绝：.. 与分隔符名内联中文原因，无副作用', async ({ page }) => {
    const scene = sceneDir()
    await resetState(page.request)
    const { picker } = await openPicker(page)

    // 非法名名单（前端预检词表与后端一致；`..` 与含分隔符名由本地预检
    // 拦截——不发必败请求）。
    const cases: [string, RegExp][] = [
      ['..', /目录名不能是「\.\.」/],
      ['a/b', /目录名不能包含路径分隔符/],
    ]
    for (const [name, reason] of cases) {
      await submitMkdir(picker, name)
      const err = picker.locator('[data-testid="mkdir-error"]')
      await expect(err).toBeVisible({ timeout: 5_000 })
      await expect(err).toContainText(reason)
      // 拒绝路径零副作用：树中无同名条目、服务端目录清单不变。
      await expect(
        picker.locator('wa-tree-item').filter({ hasText: name }),
      ).toHaveCount(0)
    }
    const listing = await page.request.get('/api/fs/browse-dirs')
    const names = ((await listing.json()) as { entries: { name: string }[] }).entries.map(
      (e) => e.name,
    )
    expect(names).not.toContain('..')
    expect(names.filter((n) => n.includes('/'))).toEqual([])

    // 取消收起命名行，弹窗保持可用。
    await picker.locator('[data-testid="mkdir-cancel"]').click()
    await expect(picker.locator('[data-testid="mkdir-name"]')).toHaveCount(0)

    expect(collector.clean()).toEqual([])
  })

  test('越界父目录被类型化拒绝（API oracle）+ 同名 400 内联呈现', async ({ page }) => {
    const scene = sceneDir()
    const t = Date.now()
    await resetState(page.request)

    // ── 越界父目录：UI 构造不出（树被 browse-dirs 边界圈死），API 直证
    // 服务端执法——父路径指到 workspace root 之外 → 400 + 中文原因，且
    // 树外零写入（路径解析先于任何 fs 写）。
    const outsideParent = path.dirname(scene)
    const evil = await page.request.post('/api/fs/mkdir', {
      data: { path: outsideParent, name: `mkdir-evil-${t}` },
    })
    expect(evil.status()).toBe(400)
    const body = (await evil.json()) as { error?: string }
    expect(body.error).toContain('路径超出根目录范围')
    expect(fs.existsSync(path.join(outsideParent, `mkdir-evil-${t}`))).toBe(false)

    // ── 同名 400：服务端类型化拒绝经 UI 内联呈现（mkdir-error 透传后端
    // message，输入行不收起）。
    const { picker } = await openPicker(page)
    const dup = `mk-dup-${t}`
    const created = await page.request.post('/api/fs/mkdir', {
      data: { path: '', name: dup },
    })
    expect(created.status()).toBe(201)
    try {
      await submitMkdir(picker, dup)
      const err = picker.locator('[data-testid="mkdir-error"]')
      await expect(err).toBeVisible({ timeout: 5_000 })
      await expect(err).toContainText(`同名目录已存在: ${dup}`)
      // 输入行保持打开（拒绝不吞操作者的输入现场）。
      await expect(picker.locator('[data-testid="mkdir-name"]')).toBeVisible()
    } finally {
      fs.rmSync(path.join(scene, dup), { recursive: true, force: true })
    }

    expect(collector.clean()).toEqual([])
  })
})
