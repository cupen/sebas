import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1000);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(800);
  await page.getByText('新建（自定义）').click();
  await sleep(800);
  console.log('=== LOCATOR DUMP (dialog fields) ===');
  const fields = await page.locator('wa-dialog input, wa-dialog select, wa-dialog wa-input, wa-dialog wa-select, wa-dialog wa-textarea, wa-dialog textarea').evaluateAll(
    (els) => els.map((e) => ({
      tag: e.tagName,
      type: e.getAttribute('type') || e.getAttribute('label') || '',
      name: e.getAttribute('name') || '',
      placeholder: e.getAttribute('placeholder') || '',
      label: e.getAttribute('label') || '',
      value: e.getAttribute('value') || '',
    }))
  );
  console.log(JSON.stringify(fields, null, 1));
  console.log('=== ARIA (dialog text) ===');
  const dlgText = await page.locator('wa-dialog').innerText();
  console.log(dlgText.slice(0, 2500));
  await shot(page, 'b12_new_provider_form');
}, 's09_new_provider_form');
