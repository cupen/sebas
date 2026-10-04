import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsModels(page) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsModels(page);

  // 1) Esc 关闭嵌套表单（round11 回归）
  await page.getByRole('button', { name: '＋ 新建（自定义）' }).click();
  await sleep(800);
  let formVisible = await page.getByLabel('名称').isVisible().catch(() => false);
  console.log('表单打开: ' + formVisible);
  await page.keyboard.press('Escape');
  await sleep(800);
  formVisible = await page.getByLabel('名称').isVisible().catch(() => false);
  console.log('Esc 后表单仍在: ' + formVisible + '（false=已关闭，round11 修复保持）');
  await shot(page, 'a39_esc_closed_form');

  // 2) 再测 Esc 关闭整个设置弹窗
  await page.keyboard.press('Escape');
  await sleep(800);
  const settingsOpen = await page.getByText('设置分区').isVisible().catch(() => false)
    || await page.getByRole('heading', { name: '设置' }).isVisible().catch(() => false);
  console.log('再 Esc 后设置弹窗仍在: ' + settingsOpen);
  await shot(page, 'a40_esc_settings');

  // 3) 别名 tab
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1000);
  await page.getByText('别名', { exact: true }).first().click();
  await sleep(1200);
  console.log('--- 别名 tab ---');
  console.log(await page.locator('[role="dialog"]').first().ariaSnapshot());
  await shot(page, 'a41_alias_tab');
}, 's14_esc_alias');
console.log('DONE');
