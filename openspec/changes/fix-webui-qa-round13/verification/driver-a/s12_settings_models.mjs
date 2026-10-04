import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1500);
  console.log('--- 设置页 ---');
  console.log((await snapshot(page)).slice(0, 2600));
  await shot(page, 'a33_settings_open');
}, 's12a_settings');
console.log('DONE');
