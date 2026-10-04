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
  // 切 Edit 档
  await page.locator('main').getByRole('combobox').click();
  await sleep(500);
  await page.getByRole('option', { name: 'Edit · 自动接受编辑' }).click();
  await sleep(1000);
  await shot(page, 'b33_switched_edit');
  const t = await page.locator('main').innerText();
  console.log('=== receipt check (Edit) ===');
  console.log(t.split('\n').slice(-8).join('\n'));
  // 发 perm
  await sendPerm(page);
  await shot(page, 'b34_edit_perm');
  const snap = await snapshot(page);
  console.log('=== edit perm state ===');
  console.log(snap.split('\n').filter(l => /批准|拒绝|审批|允许|region|button "仅|本会话|Working|Done|过程/.test(l)).join('\n'));
}, 's34_edit_mode');
