import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);

  // 登出
  await page.getByRole('button', { name: '退出登录' }).click();
  await sleep(1500);
  console.log('登出后 URL: ' + page.url());
  const s0 = await snapshot(page);
  console.log('登录页表单在: ' + (s0.includes('textbox "用户名"') && s0.includes('textbox "密码"')));
  await shot(page, 'a82_logged_out');

  // qa-live 登录 → 应被拒（已禁用）
  await page.getByRole('textbox', { name: '用户名' }).fill('qa-live');
  await page.getByRole('textbox', { name: '密码' }).fill('qa-live-2026');
  await page.getByRole('button', { name: '登录' }).click();
  await sleep(2000);
  const s1 = await snapshot(page);
  const ai = s1.indexOf('alert');
  console.log('禁用用户登录结果: ' + s1.slice(ai, ai + 120).replace(/\n/g, ' | '));
  await shot(page, 'a83_disabled_login_rejected');

  // admin 回归登录
  await page.getByRole('textbox', { name: '用户名' }).fill('admin');
  await page.getByRole('textbox', { name: '密码' }).fill('admin');
  await page.getByRole('button', { name: '登录' }).click();
  await sleep(2000);
  const s2 = await snapshot(page);
  console.log('admin 回归: ' + (s2.includes('退出 (admin · root)') ? 'OK' : '失败'));
  await shot(page, 'a84_admin_back');
}, 's21_logout_login');
console.log('DONE');
