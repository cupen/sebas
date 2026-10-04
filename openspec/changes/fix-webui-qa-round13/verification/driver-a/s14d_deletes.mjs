import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  // 删除别名
  await openSettingsTab(page, '别名');
  await page.getByRole('button', { name: '删除别名 deep13' }).click();
  await sleep(800);
  console.log('--- 删除别名确认弹窗 ---');
  const snap1 = await page.locator('body').ariaSnapshot();
  const ci = snap1.indexOf('确认');
  console.log(snap1.slice(Math.max(0, ci - 300), ci + 600));
  await shot(page, 'a44_alias_delete_confirm');
  // 找确认按钮
  const okBtn = page.getByRole('button', { name: /确认|删除|确定/ }).last();
  console.log('确认按钮: ' + JSON.stringify(await okBtn.textContent().catch(() => null)));
  await okBtn.click();
  await sleep(1200);
  const snap2 = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('删除后别名列表: ' + (snap2.includes('deep13') ? '仍在(异常)' : '已消失'));
  await shot(page, 'a45_alias_deleted');

  // 删除 provider
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(1000);
  await page.getByRole('button', { name: '🗑' }).first().click();
  await sleep(800);
  console.log('--- 删除 provider 确认弹窗 ---');
  const snap3 = await page.locator('body').ariaSnapshot();
  const ci3 = snap3.indexOf('fake13');
  console.log(snap3.slice(Math.max(0, ci3 - 400), ci3 + 500));
  await shot(page, 'a46_provider_delete_confirm');
  const okBtn2 = page.getByRole('button', { name: /确认|删除|确定/ }).last();
  console.log('确认按钮: ' + JSON.stringify(await okBtn2.textContent().catch(() => null)));
  await okBtn2.click();
  await sleep(1500);
  const snap4 = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('删除后 provider 列表: ' + (snap4.includes('fake13') ? '仍在(异常)' : '已消失') + '；' + (snap4.includes('尚未配置 provider') ? '回到空态' : ''));
  await shot(page, 'a47_provider_deleted');
}, 's14d_deletes');
console.log('DONE');
