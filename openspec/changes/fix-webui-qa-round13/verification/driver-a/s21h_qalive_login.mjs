import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  const s0 = await snapshot(page);
  if (s0.includes('退出登录')) {
    await page.getByRole('button', { name: '退出登录' }).click();
    await sleep(1800);
  }
  await page.locator('input[type="text"]').first().fill('qa-live');
  await page.locator('input[type="password"]').first().fill('qa-live-2026');
  await page.getByRole('button', { name: '登录' }).click();
  await sleep(2500);
  const s1 = await snapshot(page);
  const m = s1.match(/退出 \(([^\)]+)\)/);
  console.log('qa-live 登录: ' + (m ? '成功，头部=' + m[1] : '失败 ' + (s1.match(/alert: [^\n]+/)?.[0] || '')));
  await shot(page, 'a92_qalive_logged_in');

  // admin 角色差异面：设置 tab 集与用户管理可达性
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  const d = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('qa-live(admin) 设置 tab: ' + (d.includes('button "用户"') ? '用户 tab 可见' : '用户 tab 不可见'));
  console.log('技能同步按钮: ' + (d.includes('button "同步"') ? '可见' : '不可见'));
  await page.getByText('用户', { exact: true }).first().click();
  await sleep(1000);
  const du = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const ui = du.indexOf('heading "用户"');
  console.log('--- qa-live(admin) 用户 tab ---');
  console.log(du.slice(ui, ui + 500));
  await shot(page, 'a93_qalive_users_tab');
}, 's21h_qalive_login');
console.log('DONE');
