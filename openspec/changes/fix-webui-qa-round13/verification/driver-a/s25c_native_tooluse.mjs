import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  await page.getByRole('combobox').first().click();
  await sleep(500);
  await page.getByRole('option', { name: /Native Kernel/ }).click();
  await sleep(700);
  // 模型下拉 = 对话框第 3 个 combobox（Agent, Provider, 模型）
  const modelCombo = page.getByRole('combobox').nth(2);
  console.log('模型下拉当前: ' + JSON.stringify(await modelCombo.textContent().catch(() => '')));
  await modelCombo.click();
  await sleep(600);
  await page.getByRole('option', { name: 'test/tool-use' }).click();
  await sleep(500);
  await page.getByRole('button', { name: '创建会话' }).click();
  await sleep(2200);

  const tb = page.locator('textarea').first();
  await tb.fill('跑一个工具环');
  await page.getByRole('button', { name: '发送' }).click();
  await page.getByText(/tool_result|已执行|工具/).first().waitFor({ state: 'visible', timeout: 30000 }).catch(() => {});
  await sleep(3000);
  const s1 = await snapshot(page);
  const gi = s1.indexOf('log "会话对话"');
  console.log('--- native tool-use 回合 ---');
  console.log(s1.slice(gi, gi + 1400));
  await shot(page, 'a115_native_tooluse');
}, 's25c_tooluse');
console.log('DONE');
