import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(2000);
  console.log('URL: ' + page.url());
  const snap = await snapshot(page);
  console.log(snap);
  await shot(page, 'b39_after_restart');
}, 's37_after_restart');
