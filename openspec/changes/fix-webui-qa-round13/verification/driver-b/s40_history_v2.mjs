import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1500);
  const snap0 = await snapshot(page);
  if (snap0.includes('登录以继续')) {
    await page.getByRole('textbox', { name: '用户名' }).fill('admin');
    await page.getByRole('textbox', { name: '密码' }).fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2000);
  }
  await page.getByRole('link', { name: '历史' }).click();
  await sleep(1500);
  console.log('URL: ' + page.url());
  const snap = await snapshot(page);
  console.log(snap);
  await shot(page, 'b41_history_page');
}, 's40_history_v2');
