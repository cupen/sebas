import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(1000);
  await page.getByRole('button', { name: '＋ 新建（自定义）' }).click();
  await sleep(800);
  await page.getByLabel('名称').fill('fake13');
  await page.getByText('高级').first().click();
  await sleep(600);
  await page.getByLabel('Base URL（Anthropic）').fill('http://127.0.0.1:8791');
  await page.getByLabel('API key').fill('sk-fake13-dummy');

  // 模型清单：找「模型」区的 + 行
  const dlg = page.locator('wa-input, wa-textarea');
  const n = await dlg.count();
  for (let i = 0; i < n; i++) {
    const info = await dlg.nth(i).evaluate(e => e.tagName + '|label=' + (e.getAttribute('label') || '') + '|ph=' + (e.getAttribute('placeholder') || '')).catch(() => '');
    if (info.includes('模型') || info.includes('model')) console.log(i + ': ' + info);
  }
  await shot(page, 'a110_provider_model_rows');
}, 's24c_probe_models');
console.log('DONE');
