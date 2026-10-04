import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1200);
  await page.getByRole('listitem').filter({ hasText: 'hello' }).first().click();
  await sleep(800);
  // 审批卡还在吗（上次 pending 状态跨进程持久）
  const approve = page.getByRole('button', { name: '仅允许一次' });
  console.log('approve visible: ' + (await approve.isVisible().catch(() => false)));
  await approve.click();
  await sleep(1500);
  for (let i = 0; i < 15; i++) {
    const t = await page.locator('main').innerText();
    if (t.includes('perm turn finished') || t.includes('Done')) break;
    await sleep(1000);
  }
  await sleep(600);
  console.log('=== ARIA (after approve) ===');
  const snap = await snapshot(page);
  console.log(snap.split('\n').filter(l => /log|paragraph|text:|time|Done|Working|token|Token/.test(l)).join('\n'));
  await shot(page, 'b29_perm_ask_approved');
}, 's31_perm_approve');
