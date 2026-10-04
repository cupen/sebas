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

  const inputs = page.locator('input, select, textarea');
  const n = await inputs.count();
  console.log('input/select 总数: ' + n);
  for (let i = 0; i < n; i++) {
    const el = inputs.nth(i);
    const tag = await el.evaluate(e => e.tagName + '|' + (e.type || '') + '|' + (e.placeholder || '') + '|label=' + (e.labels?.[0]?.textContent?.trim() || '') + '|visible=' + !!(e.offsetParent));
    console.log(i + ': ' + tag);
  }
}, 's13c_inventory');
console.log('DONE');
