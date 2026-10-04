import { withPage, shot, BASE, sleep } from './helper.mjs';

const MODELS = ['test', 'test/text', 'test/long', 'test/thinking', 'test/tool-use', 'test/tools-parallel', 'test/full', 'test/empty', 'test/error'];

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
  const dlg = page.locator('wa-dialog.provider-editor');
  await dlg.locator('wa-input[label="名称"] input').fill('fake');
  await dlg.locator('wa-select[label="协议"]').click();
  await sleep(400);
  await page.locator('wa-option[value="anthropic"]').click();
  await sleep(400);
  await dlg.locator('wa-input[label="API key"] input').fill('sk-sandbox-dummy');
  await dlg.locator('wa-input[label="Base URL（Anthropic）"] input').fill('http://127.0.0.1:8791');

  const addRow = dlg.locator('.model-add, [class*="add"]').first();
  await addRow.click();
  await sleep(400);
  await dlg.locator('input[placeholder="模型 id"]').first().fill(MODELS[0]);

  for (let i = 1; i < MODELS.length; i++) {
    const plus = dlg.getByText('＋', { exact: true });
    const cnt = await plus.count();
    let clicked = false;
    for (let j = cnt - 1; j >= 0; j--) {
      const p = plus.nth(j);
      if (await p.isVisible()) { await p.click(); clicked = true; break; }
    }
    if (!clicked) throw new Error('no plus at step ' + i);
    await sleep(250);
    await dlg.locator('input[placeholder="模型 id"]').nth(i).fill(MODELS[i]);
  }
  const vals = await dlg.locator('input[placeholder="模型 id"]').evaluateAll((els) => els.map(e => e.value));
  console.log('models: ' + JSON.stringify(vals));
  const baseUrl = await dlg.locator('wa-input[label="Base URL（Anthropic）"] input').getAttribute('value') 
    ?? await dlg.locator('wa-input[label="Base URL（Anthropic）"] input').evaluate(e => e.value);
  console.log('base url: ' + baseUrl);
  await dlg.getByRole('button', { name: '保存' }).click();
  await sleep(1800);
  await shot(page, 'b17_provider_saved');
}, 's18_provider_save_v3');
