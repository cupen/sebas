import { withPage, BASE, sleep } from './helper.mjs';

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
  await dlg.locator('input[placeholder="模型 id"]').first().fill('test');
  await sleep(300);
  // dump 模型区块 HTML
  const html = await dlg.evaluate((d) => {
    // 找包含「模型 id」placeholder input 的区块
    const inp = d.querySelector('input[placeholder="模型 id"]');
    let node = inp;
    for (let i = 0; i < 4 && node; i++) node = node.parentElement;
    return node ? node.outerHTML.slice(0, 4000) : 'NOT FOUND';
  });
  console.log(html);
}, 's15_model_html');
