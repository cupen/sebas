import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  // 启用 qa-live
  await openSettingsTab(page, '用户');
  await page.getByRole('button', { name: '■' }).last().click();
  await sleep(1200);
  let s = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('重新启用: ' + (s.includes('qa-live 已启用') || s.includes('已启用')) );
  await page.keyboard.press('Escape');
  await sleep(600);

  // 登出 → qa-live 登录
  await page.getByRole('button', { name: '退出登录' }).click();
  await sleep(1500);
  await page.getByRole('textbox', { name: '用户名' }).fill('qa-live');
  await page.getByRole('textbox', { name: '密码' }).fill('qa-live-2026');
  await page.getByRole('button', { name: '登录' }).click();
  await sleep(2200);
  const s1 = await snapshot(page);
  console.log('qa-live 登录: ' + (s1.includes('退出 (qa-live · admin)') ? 'OK（头部显示 qa-live · admin）' : s1.slice(0, 200)));
  await shot(page, 'a85_qalive_logged_in');

  // admin 角色的差异面：设置页哪些 tab 可见/可操作
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  const d = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('--- qa-live（admin 角色）设置 tab ---');
  console.log(d.slice(d.indexOf('navigation "设置分区"'), d.indexOf('navigation "设置分区"') + 500));
  // 用户 tab 内容（是否可见用户列表）
  await page.getByText('用户', { exact: true }).first().click();
  await sleep(1000);
  const du = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const ui = du.indexOf('heading "用户"');
  console.log('--- qa-live 视角用户 tab ---');
  console.log(du.slice(ui, ui + 700));
  await shot(page, 'a86_qalive_users_tab');
}, 's21b_qalive');
console.log('DONE');
