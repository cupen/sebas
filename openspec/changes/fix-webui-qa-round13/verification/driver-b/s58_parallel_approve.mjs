import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1500);
  const snap0 = await snapshot(page);
  if (snap0.includes('登录以继续')) {
    await page.getByRole('textbox', { name: '用户名' }).fill('admin');
    await page.getByRole('textbox', { name: '密码' }).fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2000);
  }
  // 若审批区还在（drip 会话等待中）直接批
  let cards = 0;
  for (let k = 0; k < 4; k++) {
    const once = page.getByRole('button', { name: '仅允许一次' });
    if (await once.isVisible().catch(() => false)) {
      await once.first().click();
      cards++;
      console.log('approved card #' + cards);
      await sleep(1500);
    } else break;
  }
  for (let i = 0; i < 12; i++) {
    const s = await snapshot(page);
    if (/perm turn finished|parallel turn finished|hello world/.test(s)) break;
    await sleep(1000);
  }
  await sleep(500);
  await shot(page, 'b64_parallel_approved');
  const s = await snapshot(page);
  console.log('=== after approve both ===');
  console.log(s.split('\n').filter(l => /过程|paragraph|已执行|已拒绝|finished|Token/.test(l)).slice(0, 20).join('\n'));
}, 's58_parallel_approve');
