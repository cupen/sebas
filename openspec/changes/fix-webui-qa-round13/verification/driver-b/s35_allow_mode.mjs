import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

async function sendPerm(page) {
  await page.getByTestId('composer-input').locator('textarea').fill('perm');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(1200);
}
async function waitFinished(page) {
  for (let i = 0; i < 15; i++) {
    const t = await page.locator('main').innerText();
    if (t.includes('perm turn finished')) return true;
    await sleep(1000);
  }
  return false;
}

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1200);
  await page.getByRole('listitem').filter({ hasText: 'hello' }).first().click();
  await sleep(800);
  // 批准 edit 档的卡
  await page.getByRole('button', { name: '仅允许一次' }).click();
  console.log('edit approve -> finished: ' + (await waitFinished(page)));
  await shot(page, 'b35_edit_approved');

  // 切 Allow 档
  await page.locator('main').getByRole('combobox').click();
  await sleep(500);
  await page.getByRole('option', { name: 'Allow · 放行' }).click();
  await sleep(1000);
  await shot(page, 'b36_switched_allow');
  await sendPerm(page);
  await shot(page, 'b37_allow_perm');
  const snap = await snapshot(page);
  console.log('=== allow perm state ===');
  console.log(snap.split('\n').filter(l => /region|仅允许|拒绝|等待|✓|过程/.test(l)).join('\n'));
}, 's35_allow_mode');
