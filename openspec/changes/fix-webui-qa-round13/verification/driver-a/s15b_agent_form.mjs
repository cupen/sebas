import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsTab(page, 'Agent');
  await page.getByRole('button', { name: '＋ 新建 agent' }).click();
  await sleep(1000);
  // 枚举表单控件
  const fields = page.locator('wa-input, wa-select, wa-textarea, textarea, wa-checkbox, wa-switch');
  const n = await fields.count();
  console.log('wa 字段数: ' + n);
  for (let i = 0; i < n; i++) {
    const info = await fields.nth(i).evaluate(e => e.tagName + '|label=' + (e.getAttribute('label') || '') + '|ph=' + (e.getAttribute('placeholder') || '') + '|hint=' + (e.getAttribute('hint') || '') + '|testid=' + (e.getAttribute('data-testid') || '')).catch(e => 'err');
    console.log(i + ': ' + info);
  }
  await shot(page, 'a49_agent_form');
}, 's15b_agent_form');
console.log('DONE');
