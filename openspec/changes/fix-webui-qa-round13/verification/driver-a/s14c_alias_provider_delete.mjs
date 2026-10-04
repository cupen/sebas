import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsAlias(page) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('别名', { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsAlias(page);
  await page.getByRole('button', { name: '＋ 新建别名' }).click();
  await sleep(800);
  await page.getByTestId('alias-name-input').locator('input').fill('deep13');
  // provider 下拉默认已选 fake13（唯一 store provider），不动
  await page.getByRole('button', { name: '保存' }).click();
  await sleep(1200);
  console.log('--- 保存后别名 tab ---');
  console.log((await page.locator('[role="dialog"]').first().ariaSnapshot()).slice(0, 1600));
  await shot(page, 'a43_alias_saved');

  // 删除别名（找行上的删除按钮）
  const del = page.getByRole('button', { name: /删|🗑|delete/i });
  console.log('删除按钮数: ' + await del.count());
}, 's14c_alias_save');
console.log('DONE');
