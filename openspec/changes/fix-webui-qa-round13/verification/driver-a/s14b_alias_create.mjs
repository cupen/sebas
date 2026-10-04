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
  await sleep(900);
  // 枚举表单控件
  const inputs = page.locator('input, select, wa-select');
  const n = await inputs.count();
  console.log('控件数: ' + n);
  for (let i = 0; i < n; i++) {
    const el = inputs.nth(i);
    const info = await el.evaluate(e => e.tagName + '|' + (e.getAttribute('label') || '') + '|ph=' + (e.placeholder || e.getAttribute('placeholder') || '')).catch(e => 'err');
    console.log(i + ': ' + info);
  }
  console.log('--- 别名表单快照（第二个覆盖层）---');
  const snaps = await page.locator('wa-dialog, [role="dialog"], [class*="overlay"]').all();
  console.log('覆盖层数: ' + snaps.length);
  await shot(page, 'a42_alias_form');
  // 找 combobox（自定义下拉）里的选项
  const combos = page.getByRole('combobox');
  const cn = await combos.count();
  console.log('combobox 数: ' + cn);
  for (let i = 0; i < cn; i++) {
    console.log('combo ' + i + ': ' + JSON.stringify(await combos.nth(i).textContent().catch(() => '')));
  }
}, 's14b_alias_form');
console.log('DONE');
