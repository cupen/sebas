import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

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
  await sleep(300);

  // 「模型」标签（表单里那个，非导航）几何定位
  const modelLabel = page.locator('wa-input[label="默认模型"]');
  const bb = await modelLabel.boundingBox();
  console.log('默认模型 bbox: ' + JSON.stringify(bb));
  // 模型 add 行在「模型」小标题与「高级」之间：取 API key 输入框位置往下找
  const keyBox = await page.getByLabel('API key').boundingBox();
  console.log('API key bbox: ' + JSON.stringify(keyBox));
  if (keyBox) {
    // 模型行大约在 API key 下方 ~70px 处
    const x = keyBox.x + keyBox.width / 2;
    const y = keyBox.y + keyBox.height + 55;
    console.log('点击坐标: ' + x + ',' + y);
    await page.mouse.click(x, y);
    await sleep(900);
    const s1 = await snapshot(page);
    const mi = s1.lastIndexOf('模型');
    console.log(s1.slice(mi, mi + 900));
    await shot(page, 'a111_model_row_clicked');
  }
}, 's24g_xy');
console.log('DONE');
