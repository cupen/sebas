import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  // 项目操作菜单（切换 work_dir 的候选入口）
  await page.getByRole('button', { name: '项目操作：work' }).click();
  await sleep(600);
  console.log('=== ARIA SNAPSHOT (project menu) ===');
  console.log(await snapshot(page));
  await shot(page, 'b07_project_menu');
  await page.keyboard.press('Escape');
  await sleep(300);
  // 新建会话入口
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(1000);
  console.log('=== ARIA SNAPSHOT (new session dialog) ===');
  console.log(await snapshot(page));
  await shot(page, 'b08_new_session_dialog');
}, 's05_project_menu_new_session');
