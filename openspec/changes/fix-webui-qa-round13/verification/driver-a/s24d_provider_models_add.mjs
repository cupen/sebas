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

  // 模型 + 行：点开加号
  const plus = page.getByText('模型', { exact: true }).locator('..').getByRole('button', { name: '+' });
  const plusCount = await page.getByRole('button', { name: '+' }).count();
  console.log('+ 按钮数: ' + plusCount);
  await page.getByRole('button', { name: '+' }).first().click();
  await sleep(700);
  // 出现输入行？
  const s1 = await snapshot(page);
  const mi = s1.indexOf('模型');
  console.log(s1.slice(mi, mi + 700));
  await shot(page, 'a111_model_add_row');
}, 's24d_plus');
console.log('DONE');
