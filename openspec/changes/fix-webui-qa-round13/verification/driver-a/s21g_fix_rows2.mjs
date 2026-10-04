import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  const s0 = await snapshot(page);
  if (s0.includes('textbox "用户名"')) {
    await page.locator('input[type="text"]').first().fill('admin');
    await page.locator('input[type="password"]').first().fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2500);
  }
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('用户', { exact: true }).first().click();
  await sleep(1000);

  // 用 aria-label 找状态按钮并定位所属行
  const btns = page.getByRole('button', { name: /^(■|▶|启用|禁用)$/ });
  const n = await btns.count();
  console.log('状态按钮数: ' + n);
  const owners = [];
  for (let i = 0; i < n; i++) {
    const info = await btns.nth(i).evaluate(e => {
      let el = e;
      for (let k = 0; k < 10 && el; k++) {
        const t = (el.textContent || '').trim().replace(/\s+/g, ' ');
        if (t.includes('创建于')) return t.slice(0, 80);
        el = el.parentElement;
      }
      return '(?)';
    });
    console.log(i + ': ' + info);
    owners.push(info);
  }
  // 启用 qa-live 与 viewer（含 创建于 + 已禁用 的行）
  for (let i = 0; i < n; i++) {
    if (owners[i].includes('已禁用')) {
      const who = owners[i].split(' ')[0];
      console.log('启用 ' + who + ' …');
      await btns.nth(i).click();
      await sleep(1200);
    }
  }
  const d = await page.locator('[role="dialog"]').first().ariaSnapshot();
  for (const who of ['admin', 'member', 'viewer', 'qa-live']) {
    const wi = d.indexOf(who + ' ');
    console.log(who + ' 行: ' + d.slice(wi, wi + 60).replace(/\n/g, ' | '));
  }
  await shot(page, 'a91_users_all_enabled');
}, 's21g_fix');
console.log('DONE');
