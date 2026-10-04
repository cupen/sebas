import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsTab(page, '外观');
  await page.getByRole('button', { name: '深色 始终深色' }).click();
  await sleep(1000);
  await shot(page, 'a67_dark_theme');
  // 关弹窗看整体深色
  await page.keyboard.press('Escape');
  await sleep(800);
  await shot(page, 'a68_dark_workbench');

  // F5 持久化
  await page.reload();
  await sleep(2000);
  const s1 = await page.locator('body').ariaSnapshot();
  console.log('F5 后仍深色（外观设置里 pressed 态）: 需视觉确认');
  await shot(page, 'a69_dark_after_f5');

  // SPA 导航横切：侧栏链接点击应为 SPA 路由（navigation entry 不新增）
  const navCountBefore = await page.evaluate(() => performance.getEntriesByType('navigation').length);
  await page.getByRole('link', { name: '用量统计' }).click();
  await sleep(1200);
  const navCountAfter1 = await page.evaluate(() => performance.getEntriesByType('navigation').length);
  await page.getByRole('link', { name: 'sebas 控制台首页' }).click();
  await sleep(1200);
  const navCountAfter2 = await page.evaluate(() => performance.getEntriesByType('navigation').length);
  await page.getByRole('link', { name: '历史' }).click();
  await sleep(1200);
  const navCountAfter3 = await page.evaluate(() => performance.getEntriesByType('navigation').length);
  console.log(`navigation entries: before=${navCountBefore} 用量=${navCountAfter1} 首页=${navCountAfter2} 历史=${navCountAfter3}（全部相等=SPA 路由无整页刷新）`);
  await shot(page, 'a70_spa_nav_history');
}, 's18b_theme_spa');
console.log('DONE');
