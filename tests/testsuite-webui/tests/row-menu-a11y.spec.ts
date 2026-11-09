/**
 * Journey — 行菜单关闭态对辅助技术隐藏（fix-webui-qa-defects-round5 4.1）。
 *
 * 功能：会话管理 / 子功能：行菜单可达性
 *
 * 缺陷：的菜单项在关闭态仍暴露进 a11y 树（domSnapshot 快照里
 * 常驻「重命名/归档/移除项目」menuitem），辅助技术把不可见菜单当可用动作。
 * 修复：rail 样式表 `:not([open]) [role="menuitem"] { display:
 * none }`（open 反射属性翻转与 popup 激活同帧）。本旅程对 rail 做可访问性
 * 快照对账：关闭态不含任何行菜单项，展开态（会话行与项目行各一次）含。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  ProjectRail,
  resetState,
  waitStatus,
} from './helpers/index'

test.describe('会话管理', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('行菜单可达性', () => {
    test('closed row menus expose no menu items to the a11y tree; open ones do', async ({
      page,
    }) => {
      const rail = new ProjectRail(page)

      await resetState(page.request)
      const { name: projectName } = await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: 'a11y-menu-probe' })
      await waitStatus(page.request, key, ['done'])

      await page.goto('/')
      await rail.ensureProjectExpanded(projectName)
      const row = rail.host
        .locator('button:not(:has-text("归档于"))', { hasText: 'a11y-menu-probe' })
        .first()
      await expect(row).toBeVisible({ timeout: 10_000 })

      // 关闭态：rail 的可访问性快照不含任何行菜单项（display:none 把它们
      // 移出 a11y 树——触发钮本身仍在，动作项不可达）。
      const closed = await rail.host.ariaSnapshot()
      expect(closed).not.toContain('重命名')
      expect(closed).not.toContain('归档')
      expect(closed).not.toContain('移除项目')
      expect(closed).toContain('会话操作')
      expect(closed).toContain('项目操作')

      // 会话行菜单展开态：菜单项回到 a11y 树。AntD 菜单渲染到 body 的
      // portal，不在 rail 子树内——开放态按页面级 a11y 树对账。
      await rail.openSessionMenu('a11y-menu-probe')
      const renameItem = page.getByRole('menuitem', { name: '重命名会话' })
      await expect(renameItem).toBeVisible()
      const sessionOpen = await page.locator('body').ariaSnapshot()
      expect(sessionOpen).toContain('重命名')
      expect(sessionOpen).toContain('归档')

      // 收起（Escape）：菜单项再次从 a11y 树消失——不是单向隐藏。关闭后
      // portal 残壳可能短暂挂载，因此对页面级树断言菜单项文本缺席。
      await page.keyboard.press('Escape')
      await expect(renameItem).toBeHidden()
      const reclosed = await page.locator('body').ariaSnapshot()
      expect(reclosed).not.toContain('重命名')
      expect(reclosed).not.toContain('归档')

      // 项目行菜单展开态：移除项目动作同理——关闭态缺席、展开态在场。
      await rail.openProjectMenu(projectName)
      const removeItem = page.getByRole('menuitem', { name: '移除项目' })
      await expect(removeItem).toBeVisible()
      const projectOpen = await page.locator('body').ariaSnapshot()
      expect(projectOpen).toContain('移除项目')
      await page.keyboard.press('Escape')
      await expect(removeItem).toBeHidden()
      expect(await page.locator('body').ariaSnapshot()).not.toContain('移除项目')

      expect(collector.clean()).toEqual([])
    })
  })
})
