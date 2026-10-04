import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1000);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(1000);
  console.log('=== ARIA SNAPSHOT (models tab) ===');
  console.log(await snapshot(page));
  await shot(page, 'b11_settings_models');
}, 's08_settings_models');
