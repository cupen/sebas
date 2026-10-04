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
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  await page.getByRole('combobox', { name: 'Agent' }).click();
  await sleep(600);
  const snap = await snapshot(page);
  console.log(snap.split('\n').filter(l => /option|combobox|dialog|heading/.test(l)).join('\n'));
  await shot(page, 'b43_agent_dropdown_after_fix');
}, 's42_native_recheck');
