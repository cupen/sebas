import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

const AGENTS = ['thinking', 'empty', 'error', 'slow', 'claude'];

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
  for (const ag of AGENTS) {
    await page.getByRole('button', { name: '在 work 中新建会话' }).click();
    await sleep(800);
    await page.getByRole('combobox', { name: 'Agent' }).click();
    await sleep(500);
    await page.getByRole('option', { name: ag, exact: true }).click();
    await sleep(500);
    await page.getByRole('button', { name: '创建会话' }).click();
    await sleep(1500);
    console.log('created: ' + ag);
  }
  const snap = await snapshot(page);
  console.log(snap.split('\n').filter(l => /listitem|text:/.test(l)).slice(0, 30).join('\n'));
  await shot(page, 'b44_five_sessions');
}, 's43_create_scenarios');
