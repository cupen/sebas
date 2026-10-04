import { withPage, shot, BASE, sleep } from './helper.mjs';

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

  // 点「+」行展开模型编辑
  const addRow = dlg.locator('.model-add, [class*="add"]').first();
  await addRow.click();
  await sleep(600);
  // dump 所有可见 input 的 placeholder 与 value
  const inputs = await dlg.locator('input').evaluateAll((els) =>
    els.map(e => ({ ph: e.placeholder, vis: e.getBoundingClientRect().width > 0, val: e.value }))
  );
  console.log('inputs: ' + JSON.stringify(inputs, null, 1));
  // dump checkbox / toggle 形态
  const boxes = await dlg.locator('wa-checkbox, wa-switch, wa-radio').evaluateAll((els) =>
    els.map(e => ({ label: e.getAttribute('label') || e.textContent.trim(), vis: e.getBoundingClientRect().width > 0 }))
  );
  console.log('toggles: ' + JSON.stringify(boxes));
  await shot(page, 'b15_model_row_editor');
}, 's12_model_row_probe');
