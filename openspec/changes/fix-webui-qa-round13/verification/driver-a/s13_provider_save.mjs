import { withPage, shot, sleep, BASE } from './helper-a.mjs';

const DIALOG = '[role="dialog"]';
async function openSettingsModels(page) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsModels(page);
  await page.getByRole('button', { name: '＋ 新建（自定义）' }).click();
  await sleep(1000);

  // 表单 = 第二个 dialog
  const form = page.locator(DIALOG).last();
  console.log(await form.ariaSnapshot());
  await shot(page, 'a35_provider_form');
}, 's13a_form_dump');
console.log('DONE');
