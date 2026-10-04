import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2000);
  if ((await page.getByRole('textbox', { name: '用户名' }).count()) === 1) {
    await page.getByRole('textbox', { name: '用户名' }).fill('admin');
    await page.getByRole('textbox', { name: '密码' }).fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2000);
    console.log('after login URL: ' + page.url());
  }
  await sleep(1200);

  // 1) native test/thinking 会话：建会话 → 发消息 → 等 thinking chip → 展开
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  const agentCombo = page.getByRole('combobox').first();
  await agentCombo.click();
  await sleep(500);
  await page.getByRole('option', { name: /Native Kernel/ }).click();
  await sleep(700);
  const modelCombo = page.getByRole('combobox', { name: '模型' });
  await modelCombo.click();
  await sleep(600);
  await page.getByRole('option', { name: 'test/thinking' }).click();
  await sleep(500);
  await page.getByRole('button', { name: '创建会话' }).click();
  await sleep(2200);

  const tb = page.locator('textarea').first();
  await tb.fill('round13 抽查：深想一下');
  await page.getByRole('button', { name: '发送' }).click();
  await page.getByText(/thinking/i).first().waitFor({ state: 'visible', timeout: 30000 }).catch(() => {});
  await sleep(3000);
  // 点击 thinking 过程 chip 展开
  const chip = page.getByText(/^过程 thinking/i).first();
  const chipAlt = page.getByText(/thinking/i).first();
  if (await chip.count()) { await chip.click(); } else { await chipAlt.click(); }
  await sleep(800);
  await shot(page, 'r13_thinking_fixed');
  const s1 = await snapshot(page);
  const gi = s1.indexOf('会话对话');
  console.log('--- thinking 展开态 ---');
  console.log(s1.slice(Math.max(0, gi), gi + 900));

  // 2) usage 页：刻度 + 数据源说明行
  await page.getByRole('link', { name: '用量' }).click();
  await sleep(2000);
  await shot(page, 'r13_usage_source_note');
  const s2 = await snapshot(page);
  const ui = s2.indexOf('用量');
  console.log('--- usage 页 ---');
  console.log(s2.slice(Math.max(0, ui), ui + 1200));
}, 'r13_check');
console.log('DONE');
