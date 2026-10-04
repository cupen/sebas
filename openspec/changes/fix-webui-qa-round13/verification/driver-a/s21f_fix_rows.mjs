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

  // 枚举每个 状态按钮（■/▶）所属行
  const btns = page.locator('button').filter({ hasText: /^(■|▶)$/ });
  const n = await btns.count();
  console.log('状态按钮数: ' + n);
  for (let i = 0; i < n; i++) {
    const owner = await btns.nth(i).evaluate(e => {
      let el = e;
      for (let k = 0; k < 8 && el; k++) {
        const t = (el.textContent || '').trim();
        if (t.length > 5) return t.slice(0, 60).replace(/\s+/g, ' ');
        el = el.parentElement;
      }
      return '(?)';
    });
    console.log(i + ': ' + owner);
  }
  await shot(page, 'a90_user_rows_before_fix');
}, 's21f_scan');
console.log('DONE');
