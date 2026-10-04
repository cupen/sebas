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
  await sendPerm(page);
  const reject = page.getByRole('button', { name: '拒绝' });
  console.log('reject visible: ' + (await reject.isVisible().catch(() => false)));
  await shot(page, 'b30_before_reject');
  await reject.click();
  await sleep(2000);
  for (let i = 0; i < 10; i++) {
    const t = await page.locator('main').innerText();
    if (t.includes('拒绝') || t.includes('rejected') || t.includes('denied')) break;
    await sleep(1000);
  }
  await sleep(500);
  console.log('=== ARIA (after reject) ===');
  const snap = await snapshot(page);
  console.log(snap.split('\n').filter(l => /paragraph|拒绝|reject|denied|Token|text: ✓/.test(l)).join('\n'));
  await shot(page, 'b31_perm_rejected');
}, 's32_perm_reject');
