import { withPage, snapshot, sleep, BASE } from './helper-a.mjs';
await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2000);
  if ((await page.getByRole('textbox', { name: '用户名' }).count()) === 1) {
    await page.getByRole('textbox', { name: '用户名' }).fill('admin');
    await page.getByRole('textbox', { name: '密码' }).fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2000);
  }
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  const agentCombo = page.getByRole('combobox').first();
  await agentCombo.click();
  await sleep(500);
  await page.getByRole('option', { name: /Native Kernel/ }).click();
  await sleep(700);
  const combos = page.getByRole('combobox');
  console.log('combos=' + (await combos.count()));
  const mc = combos.nth((await combos.count()) - 2);
  await mc.click();
  await sleep(700);
  const s = await snapshot(page);
  const di = s.indexOf('dialog');
  console.log(s.slice(Math.max(0, di), di + 1500));
}, 'r13_diag2');
