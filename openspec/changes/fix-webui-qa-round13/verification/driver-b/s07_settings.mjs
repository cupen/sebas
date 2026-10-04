import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  console.log('URL: ' + page.url());
  console.log('=== ARIA SNAPSHOT (settings) ===');
  console.log(await snapshot(page));
  await shot(page, 'b10_settings');
}, 's07_settings');
