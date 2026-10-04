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
  await openSettingsTab(page, '用户');
  // qa-live 是最后一行 → 最后一个 combobox
  const combo = page.getByRole('combobox').last();
  console.log('最后 combobox aria: ' + JSON.stringify(await combo.getAttribute('aria-label').catch(() => null)));
  await combo.click();
  await sleep(700);
  const s0 = await snapshot(page);
  const li = s0.indexOf('listbox');
  console.log('--- 角色下拉 ---');
  console.log(s0.slice(li, li + 400));
  await shot(page, 'a79_role_menu');
  await page.getByRole('option', { name: 'admin' }).click();
  await sleep(1500);
  let s = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const qline = s.indexOf('qa-live');
  console.log('改角色后 qa-live 行: ' + s.slice(qline, qline + 160).replace(/\n/g, ' | '));
  await shot(page, 'a80_user_role_admin');

  // 禁用 qa-live（最后一行的 ■）
  const disBtns = page.getByRole('button', { name: '■' });
  console.log('■ 数: ' + await disBtns.count());
  await disBtns.last().click();
  await sleep(1200);
  s = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const q2 = s.indexOf('qa-live');
  console.log('禁用后 qa-live 行: ' + s.slice(q2, q2 + 160).replace(/\n/g, ' | '));
  await shot(page, 'a81_user_disabled');
}, 's20e_role_disable');
console.log('DONE');
