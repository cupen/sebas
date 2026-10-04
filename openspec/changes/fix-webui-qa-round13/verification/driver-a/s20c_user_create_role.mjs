import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsTab(page, '用户');
  await page.getByRole('button', { name: '＋ 新建用户' }).click();
  await sleep(800);
  await page.getByLabel('用户名').locator('input').fill('qa-live');
  await page.getByLabel(/密码/).locator('input').fill('qa-live-2026');
  await page.getByRole('button', { name: '创建' }).click();
  await sleep(1200);
  let s = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('qa-live 已建: ' + s.includes('qa-live'));
  await shot(page, 'a78_user_created');

  // 改角色 member → admin（qa-live 行的 combobox）
  const combo = page.getByRole('combobox').filter({ hasText: 'member' }).first();
  await combo.click();
  await sleep(600);
  const s0 = await snapshot(page);
  const li = s0.indexOf('listbox');
  console.log('--- 角色下拉 ---');
  console.log(s0.slice(li, li + 400));
  await page.getByRole('option', { name: 'admin' }).click();
  await sleep(1200);
  s = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const qline = s.indexOf('qa-live');
  console.log('改角色后 qa-live 行: ' + s.slice(qline, qline + 120).replace(/\n/g, ' | '));
  await shot(page, 'a79_user_role_changed');
}, 's20c_create_role');
console.log('DONE');
