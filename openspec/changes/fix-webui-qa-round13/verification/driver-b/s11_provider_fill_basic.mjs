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

  // 名称
  await dlg.locator('wa-input[label="名称"] input').fill('fake');
  // 协议 → Anthropic
  await dlg.locator('wa-select[label="协议"]').click();
  await sleep(500);
  await page.locator('wa-option[value="anthropic"]').click();
  await sleep(500);
  // API key
  await dlg.locator('wa-input[label="API key"] input').fill('sk-sandbox-dummy');
  await shot(page, 'b14_provider_basic_filled');

  // 模型列表：+ 展开输入行
  const modelPlus = dlg.locator('text=模型').locator('..').locator('button, wa-button').filter({ hasText: '+' });
  // 更稳妥：找「模型」区块里的加号行（截图显示为虚线行）
  const addModelRow = dlg.locator('.model-add, [class*="add"]').first();
  console.log('model add row count: ' + (await addModelRow.count()));
  if (await addModelRow.count()) {
    await addModelRow.click();
    await sleep(500);
    const inputs = dlg.locator('input[placeholder*="模型"], input[placeholder*="model"], input[placeholder*="id"]');
    console.log('model input count: ' + (await inputs.count()));
  }
  console.log('=== dialog text now ===');
  console.log((await dlg.innerText()).slice(0, 1800));
}, 's11_provider_fill_basic');
