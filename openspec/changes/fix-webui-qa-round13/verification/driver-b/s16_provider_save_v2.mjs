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

  const addRow = dlg.locator('.model-add, [class*="add"]').first();
  await addRow.click();
  await sleep(500);
  await dlg.locator('input[placeholder="模型 id"]').first().fill(MODELS[0]);
  await sleep(200);

  for (let i = 1; i < MODELS.length; i++) {
    const plus = dlg.getByText('＋', { exact: true });
    const cnt = await plus.count();
    console.log(`step ${i}: plus count=${cnt}`);
    // 点最后一个可见的 ＋
    let clicked = false;
    for (let j = cnt - 1; j >= 0; j--) {
      const p = plus.nth(j);
      if (await p.isVisible()) {
        await p.click();
        clicked = true;
        break;
      }
    }
    if (!clicked) throw new Error('no visible plus found at step ' + i);
    await sleep(300);
    const rows = dlg.locator('input[placeholder="模型 id"]');
    console.log('  model inputs now: ' + (await rows.count()));
    await rows.nth(i).fill(MODELS[i]);
  }
  await shot(page, 'b16_provider_models_filled');
  console.log('=== all model values ===');
  const vals = await dlg.locator('input[placeholder="模型 id"]').evaluateAll((els) => els.map(e => e.value));
  console.log(JSON.stringify(vals));
  await dlg.getByRole('button', { name: '保存' }).click();
  await sleep(1500);
  const settingsDlg = page.locator('wa-dialog').filter({ hasText: '管理模型 provider' });
  console.log('=== after save ===');
  console.log((await settingsDlg.innerText()).slice(0, 1200));
  await shot(page, 'b17_provider_saved');
}, 's16_provider_save_v2');
