import { withPage, shot, sleep, BASE } from './helper-a.mjs';

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
export const DIALOG = '[role="dialog"]';

await withPage(async (page) => {
  await openSettings(page, '模型');
  console.log(await page.locator(DIALOG).ariaSnapshot());
  await shot(page, 'a34_settings_models_tab');
}, 's12d_dialog');
console.log('DONE');
