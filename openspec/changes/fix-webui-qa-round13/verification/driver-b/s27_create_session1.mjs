import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  await page.getByRole('button', { name: '创建会话' }).click();
  await sleep(2500);
  console.log('=== ARIA (after create S1) ===');
  const snap = await snapshot(page);
  console.log(snap);
  await shot(page, 'b23_session1_created');
}, 's27_create_session1');
