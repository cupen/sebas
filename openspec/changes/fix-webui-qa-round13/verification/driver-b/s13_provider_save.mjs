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

  for (let i = 0; i < MODELS.length; i++) {
    const rows = dlg.locator('input[placeholder="模型 id"]');
    const n = await rows.count();
    await rows.nth(n - 1).fill(MODELS[i]);
    if (i < MODELS.length - 1) {
      // 点虚线 + 行加下一行
      const plusRow = dlg.locator('.model-add, [class*="add"]').first();
      await plusRow.click();
      await sleep(300);
    }
  }
  await shot(page, 'b16_provider_models_filled');
  await dlg.getByRole('button', { name: '保存' }).click();
  await sleep(1500);
  console.log('=== dialog text after save ===');
  const settingsDlg = page.locator('wa-dialog').filter({ hasText: '管理模型 provider' });
  console.log((await settingsDlg.innerText()).slice(0, 1200));
  await shot(page, 'b17_provider_saved');
}, 's13_provider_save');
