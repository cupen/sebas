import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsTab(page, '外观');
  console.log('--- 外观 tab ---');
  console.log(await page.locator('[role="dialog"]').first().ariaSnapshot());
  await shot(page, 'a66_appearance_tab');
}, 's18a_appearance');
console.log('DONE');
