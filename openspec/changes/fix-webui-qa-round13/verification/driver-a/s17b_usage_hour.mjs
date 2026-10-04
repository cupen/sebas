import { withPage, shot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/usage');
  await sleep(1800);
  await page.getByRole('button', { name: '按小时（今天）' }).click();
  await sleep(1800);
  console.log((await page.locator('body').ariaSnapshot()).match(/img "折线图[^"]*"/)?.[0]);
  console.log((await page.locator('body').ariaSnapshot()).includes('桶边界按本机时区'));
  await shot(page, 'a64_usage_hourly');
  // 输入维度切换
  await page.getByRole('button', { name: '输入' }).click();
  await sleep(1200);
  await shot(page, 'a65_usage_input_dim');
}, 's17b_usage_hour');
console.log('DONE');
