import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('link', { name: '用量统计' }).click();
  await sleep(2000);
  console.log('URL: ' + page.url());
  console.log('--- usage 页 ---');
  console.log((await page.locator('body').ariaSnapshot()).slice(0, 3000));
  await shot(page, 'a63_usage_hour');
}, 's17a_usage');
console.log('DONE');
