import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('textbox', { name: '用户名' }).fill('admin');
  await page.getByRole('textbox', { name: '密码' }).fill('admin');
  await shot(page, 'b02_login_filled');
  await page.getByRole('button', { name: '登录' }).click();
  await sleep(2500);
  console.log('URL after login: ' + page.url());
  console.log('=== ARIA SNAPSHOT (workbench) ===');
  console.log(await snapshot(page));
  await shot(page, 'b03_workbench_initial');
}, 's02_login');
