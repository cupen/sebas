import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1200);
  await page.getByRole('listitem').filter({ hasText: 'hello' }).first().click();
  await sleep(800);
  await page.keyboard.press('Escape');
  // C.10 ask 档（当前 Ask·逐次询问）发 perm
  await page.getByTestId('composer-input').locator('textarea').fill('perm');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(1500);
  await shot(page, 'b28_perm_ask_card');
  const snap = await snapshot(page);
  console.log('=== ask perm state ===');
  console.log(snap.split('\n').filter(l => /批准|拒绝|审批|权限|tool|允许|button|Working|Done/.test(l)).join('\n'));
}, 's30_perm_ask');
