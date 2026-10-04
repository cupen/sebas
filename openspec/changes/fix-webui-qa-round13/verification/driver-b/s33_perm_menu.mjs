import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

async function sendPerm(page) {
  await page.getByTestId('composer-input').locator('textarea').fill('perm');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(1200);
}

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1200);
  await page.getByRole('listitem').filter({ hasText: 'hello' }).first().click();
  await sleep(800);
  // 打开权限模式 combobox 看选项
  await page.locator('main').getByRole('combobox').click();
  await sleep(600);
  const snap1 = await snapshot(page);
  console.log('=== perm mode options ===');
  console.log(snap1.split('\n').filter(l => /option|listbox/.test(l)).join('\n'));
  await shot(page, 'b32_perm_mode_menu');
}, 's33_perm_menu');
