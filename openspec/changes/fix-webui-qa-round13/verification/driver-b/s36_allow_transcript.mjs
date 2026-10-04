import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1200);
  await page.getByRole('listitem').filter({ hasText: 'hello' }).first().click();
  await sleep(1000);
  const t = await page.locator('main').innerText();
  console.log('=== full transcript ===');
  console.log(t);
  await shot(page, 'b38_allow_result_full');
}, 's36_allow_transcript');
