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

  // 名称
  const nameBox = page.getByPlaceholder(/名称|name/i).first();
  console.log('名称框数: ' + await page.getByPlaceholder(/名称/i).count());
  await nameBox.fill('fake13');
  // 协议：找 select 元素
  const selects = page.locator('select');
  console.log('select 数: ' + await selects.count());
  if (await selects.count() > 0) {
    const opts = await selects.first().locator('option').allTextContents();
    console.log('协议选项: ' + JSON.stringify(opts));
    await selects.first().selectOption({ label: opts.find(o => o.includes('Anthropic')) || opts[1] });
    await sleep(600);
  }
  await shot(page, 'a36_provider_form_anthropic');
  // Base URL（协议切了以后 label 可能变）
  const urlBox = page.getByPlaceholder(/endpoint|url/i).first();
  await urlBox.fill('http://127.0.0.1:8791');
  // API key
  await page.getByPlaceholder(/API key/i).first().fill('sk-fake13-dummy');
  await sleep(400);
  console.log('--- 填完 ---');
  await shot(page, 'a37_provider_form_filled');
  // 保存
  await page.getByRole('button', { name: '保存' }).click();
  await sleep(1500);
  console.log('--- 保存后设置弹窗 ---');
  console.log(await page.locator('[role="dialog"]').first().ariaSnapshot());
  await shot(page, 'a38_provider_saved');
}, 's13b_fill_save');
console.log('DONE');
