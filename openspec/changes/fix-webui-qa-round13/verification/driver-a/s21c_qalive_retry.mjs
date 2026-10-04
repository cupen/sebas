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
  // admin 确认 qa-live 状态
  await page.getByRole('textbox', { name: '用户名' }).fill('admin');
  await page.getByRole('textbox', { name: '密码' }).fill('admin');
  await page.getByRole('button', { name: '登录' }).click();
  await sleep(2000);
  await openSettingsTab(page, '用户');
  const s0 = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const qi = s0.indexOf('qa-live');
  console.log('qa-live 当前状态行: ' + s0.slice(qi, qi + 200).replace(/\n/g, ' | '));
  await shot(page, 'a87_qalive_state_check');

  // 若已禁用则再点启用
  if (s0.slice(qi, qi + 200).includes('已禁用')) {
    console.log('仍是禁用态 → 再点启用');
    await page.getByRole('button', { name: '■' }).last().click();
    await sleep(1500);
    const s1 = await page.locator('[role="dialog"]').first().ariaSnapshot();
    const q2 = s1.indexOf('qa-live');
    console.log('再启用后: ' + s1.slice(q2, q2 + 160).replace(/\n/g, ' | '));
    await shot(page, 'a88_qalive_reenabled');
  }
  await page.keyboard.press('Escape');
  await sleep(500);
  // 登出 → qa-live 第二次登录尝试
  await page.getByRole('button', { name: '退出登录' }).click();
  await sleep(1500);
  await page.getByRole('textbox', { name: '用户名' }).fill('qa-live');
  await page.getByRole('textbox', { name: '密码' }).fill('qa-live-2026');
  await page.getByRole('button', { name: '登录' }).click();
  await sleep(2500);
  const s2 = await snapshot(page);
  console.log('qa-live 第二次登录: ' + (s2.includes('退出 (qa-live') ? '成功 ' + s2.match(/退出 \(qa-live[^\)]*\)/)?.[0] : '仍失败: ' + (s2.match(/alert: [^\n]+/)?.[0] || '')));
  await shot(page, 'a89_qalive_login_retry2');
}, 's21c_retry');
console.log('DONE');
