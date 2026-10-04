import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  console.log('URL: ' + page.url());
  console.log('--- snapshot login page ---');
  console.log(await snapshot(page));
  await shot(page, 'a01_login_initial');

  // 错误密码
  const userField = page.locator('input[type="text"], input[name*="user" i], input[placeholder*="用户" i]').first();
  const passField = page.locator('input[type="password"]').first();
  await userField.fill('admin');
  await passField.fill('wrongpass');
  await shot(page, 'a02_login_wrong_filled');
  // 找提交按钮
  const btn = page.getByRole('button').first();
  console.log('button text: ' + JSON.stringify(await btn.textContent().catch(() => null)));
  await btn.click();
  await sleep(2000);
  console.log('--- snapshot after wrong password ---');
  console.log(await snapshot(page));
  await shot(page, 'a03_login_wrong_error');

  // 正确登录
  await passField.fill('admin');
  await btn.click();
  await sleep(2500);
  console.log('URL after login: ' + page.url());
  console.log('--- snapshot after login ---');
  console.log(await snapshot(page));
  await shot(page, 'a04_login_success_workbench');
}, 'a01_login');
console.log('DONE');
