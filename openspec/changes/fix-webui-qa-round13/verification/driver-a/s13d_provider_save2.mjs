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
  await page.getByRole('button', { name: '＋ 新建（自定义）' }).click();
  await sleep(1000);

  await page.getByLabel('名称').fill('fake13');
  // 展开「高级」折叠区（Anthropic/Responses URL 在里面）
  await page.getByText('高级').first().click();
  await sleep(800);
  await page.getByLabel('Base URL（Anthropic）').fill('http://127.0.0.1:8791');
  await page.getByLabel('API key').fill('sk-fake13-dummy');
  await sleep(300);
  await shot(page, 'a37_provider_form_filled');
  await page.getByRole('button', { name: '保存' }).click();
  await sleep(1800);
  console.log('--- 保存后 ---');
  console.log(await page.locator('[role="dialog"]').first().ariaSnapshot());
  await shot(page, 'a38_provider_saved');
}, 's13d_save');
console.log('DONE');
