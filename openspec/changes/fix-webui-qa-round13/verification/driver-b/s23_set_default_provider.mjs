import { withPage, shot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1000);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(800);
  const modal = page.locator('sebas-settings-modal');
  await modal.getByRole('button', { name: '设为新建会话的默认' }).click();
  await sleep(1000);
  await shot(page, 'b19_default_set');
  // 关闭设置
  await modal.getByRole('button', { name: '关闭设置' }).click();
  await sleep(600);
  // 重新打开新建会话
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(800);
  await page.getByRole('combobox', { name: 'Agent' }).click();
  await sleep(500);
  const snap = await snapshot(page);
  console.log(snap.split('\n').filter(l => /option|listbox|combobox|dialog|heading/.test(l)).join('\n'));
  await shot(page, 'b20_agent_dropdown_default');
}, 's23_set_default_provider');
