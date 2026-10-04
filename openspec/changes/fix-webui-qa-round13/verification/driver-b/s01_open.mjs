import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1200);
  console.log('URL: ' + page.url());
  console.log('TITLE: ' + (await page.title()));
  console.log('=== ARIA SNAPSHOT (initial) ===');
  console.log(await snapshot(page));
  await shot(page, 'b01_login_page');
}, 's01_open');
