import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  const s0 = await snapshot(page);
  if (!s0.includes('退出 (admin')) {
    if (s0.includes('退出登录')) { await page.getByRole('button', { name: '退出登录' }).click(); await sleep(1800); }
    await page.locator('input[type="text"]').first().fill('admin');
    await page.locator('input[type="password"]').first().fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2500);
  }

  // 新建 native 会话：Agent=Native Kernel，看模型下拉
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  const agentCombo = page.getByRole('combobox').first();
  await agentCombo.click();
  await sleep(500);
  await page.getByRole('option', { name: /Native Kernel/ }).click();
  await sleep(800);
  const d = await snapshot(page);
  const di = d.indexOf('dialog');
  console.log('--- native 会话对话框 ---');
  console.log(d.slice(di, di + 1500));
  await shot(page, 'a107_native_dialog');
}, 's24a_native_dialog');
console.log('DONE');
