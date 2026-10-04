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

  const entries = page.getByTestId('model-entry');
  console.log('model-entry 数: ' + await entries.count());
  const fillN = async (i, v) => entries.nth(i).locator('input[type="text"]').fill(v);
  await fillN(0, 'test/thinking');
  // + 行：model-entry 下方
  let b = await entries.nth(0).boundingBox();
  await page.mouse.click(b.x + b.width / 2, b.y + b.height + 22);
  await sleep(600);
  console.log('加行后: ' + await entries.count());
  await fillN(1, 'test/tool-use');
  b = await entries.nth(1).boundingBox();
  await page.mouse.click(b.x + b.width / 2, b.y + b.height + 22);
  await sleep(600);
  console.log('加行后2: ' + await entries.count());
  await fillN(2, 'test/long');
  await sleep(300);
  await shot(page, 'a112_models_filled');
  await page.getByRole('button', { name: '保存' }).click();
  await sleep(1800);
  const d = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const fi = d.indexOf('fake13');
  console.log('fake13 行: ' + d.slice(fi, fi + 260).replace(/\n/g, ' | '));
  await shot(page, 'a113_provider_with_models');
}, 's24i_fill2');
console.log('DONE');
