import { withPage, shot, sleep, BASE } from './helper-a.mjs';

const DIALOG = '[role="dialog"]';
async function openSettings(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  if (tab) {
    await page.getByText(tab, { exact: true }).first().click();
    await sleep(1000);
  }
}

await withPage(async (page) => {
  await openSettings(page, '模型');
  await page.getByRole('button', { name: '＋ 新建（自定义）' }).click();
  await sleep(1000);
  console.log(await page.locator(DIALOG).ariaSnapshot());
  await shot(page, 'a35_provider_form');
}, 's12e_form');
console.log('DONE');
