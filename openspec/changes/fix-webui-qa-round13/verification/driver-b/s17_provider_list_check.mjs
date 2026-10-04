import { withPage, shot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1000);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(1000);
  await shot(page, 'b17b_provider_list_check');
  const settingsDlg = page.locator('wa-dialog').filter({ hasText: '管理模型 provider' });
  console.log('settings dlg count: ' + (await settingsDlg.count()));
  const txt = await page.locator('wa-dialog.provider-editor').count();
  console.log('editor dlg count: ' + txt);
  // dump 所有 open 的 wa-dialog label
  const labels = await page.locator('wa-dialog').evaluateAll((els) => els.filter(e => e.hasAttribute('open')).map(e => e.getAttribute('label')));
  console.log('open dialogs: ' + JSON.stringify(labels));
  // provider 列表区文本
  const body = await page.evaluate(() => document.body.innerText);
  console.log('=== body text (contains provider list?) ===');
  console.log(body.slice(0, 1500));
}, 's17_provider_list_check');
