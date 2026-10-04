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
  console.log('editor open: ' + (await dlg.count()));
  await shot(page, 'b12_new_provider_form');

  // 协议下拉：看选项
  const proto = dlg.locator('wa-select').first();
  await proto.click();
  await sleep(600);
  const opts = await dlg.locator('wa-option').evaluateAll((els) =>
    els.filter(e => e.offsetParent !== null || e.getBoundingClientRect().width > 0).map(e => e.textContent.trim() + ' | value=' + (e.getAttribute('value') || ''))
  );
  console.log('protocol options: ' + JSON.stringify(opts));
  await shot(page, 'b13_protocol_options');
}, 's10_protocol_dump');
