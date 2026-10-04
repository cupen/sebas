import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';
await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2000);
  if ((await page.getByRole('textbox', { name: '用户名' }).count()) === 1) {
    await page.getByRole('textbox', { name: '用户名' }).fill('admin');
    await page.getByRole('textbox', { name: '密码' }).fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2000);
  }
  await page.getByText('round13 抽查：深想一下').first().click();
  await sleep(2000);
  const chip = page.getByText(/过程\s*thinking/i).first();
  console.log('chip count=' + (await chip.count()));
  await chip.click();
  await sleep(900);
  await shot(page, 'r13_thinking_expanded');
  const s = await snapshot(page);
  const gi = s.indexOf('会话对话');
  console.log(s.slice(Math.max(0, gi), gi + 800));
}, 'r13_think2');
