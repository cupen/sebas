import { withPage, shot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(1000);
  await page.getByRole('button', { name: '＋ 新建（自定义）' }).click();
  await sleep(800);
  await page.getByLabel('名称').fill('fake13');
  await page.getByText('高级').first().click();
  await sleep(500);
  await page.getByLabel('Base URL（Anthropic）').fill('http://127.0.0.1:8791');
  await page.getByLabel('API key').fill('sk-fake13-dummy');
  const keyBox = await page.getByLabel('API key').boundingBox();
  await page.mouse.click(keyBox.x + keyBox.width / 2, keyBox.y + keyBox.height + 55);
  await sleep(800);

  // 第一行模型 id
  const rowInput = page.getByPlaceholder('模型 id');
  console.log('模型 id 行数: ' + await rowInput.count());
  await rowInput.fill('test/thinking');
  // 加第二行：点下方 + 行（第一行下方 ~46px）
  let rowBox = await rowInput.first().boundingBox();
  await page.mouse.click(rowBox.x + rowBox.width / 2, rowBox.y + rowBox.height + 24);
  await sleep(700);
  console.log('第二行后行数: ' + await rowInput.count());
  await rowInput.nth(1).fill('test/tool-use');
  rowBox = await rowInput.nth(1).boundingBox();
  await page.mouse.click(rowBox.x + rowBox.width / 2, rowBox.y + rowBox.height + 24);
  await sleep(700);
  console.log('第三行后行数: ' + await rowInput.count());
  await rowInput.nth(2).fill('test/long');
  await sleep(300);
  await shot(page, 'a112_models_filled');
  await page.getByRole('button', { name: '保存' }).click();
  await sleep(1500);
  const d = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('保存后 fake13 行: ' + (d.includes('fake13') ? '在' : '丢失'));
  console.log(d.slice(d.indexOf('fake13'), d.indexOf('fake13') + 160).replace(/\n/g, ' | '));
  await shot(page, 'a113_provider_with_models');
}, 's24h_fill');
console.log('DONE');
