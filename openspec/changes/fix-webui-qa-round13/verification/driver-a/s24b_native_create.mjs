import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  // 建 native 会话
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  const agentCombo = page.getByRole('combobox').first();
  await agentCombo.click();
  await sleep(500);
  await page.getByRole('option', { name: /Native Kernel/ }).click();
  await sleep(600);
  await page.getByRole('button', { name: '创建会话' }).click();
  await sleep(2000);

  // 发一条消息（默认模型 = env SEBAS_AGENT_MODEL=test）
  const tb = page.locator('textarea').first();
  await tb.fill('native default probe');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(3500);
  const s1 = await snapshot(page);
  const gi = s1.indexOf('log "会话对话"');
  console.log('--- native 默认回合 ---');
  console.log(s1.slice(gi, gi + 900));
  await shot(page, 'a108_native_default_turn');

  // 模型 chip（composer 右下）
  const chip = page.getByRole('button', { name: /^(test|default|fake)$/ }).last();
  console.log('模型 chip 文本: ' + JSON.stringify(await chip.textContent().catch(() => null)));
  await chip.click();
  await sleep(900);
  const s2 = await snapshot(page);
  const mi = s2.indexOf('listbox');
  console.log('--- 模型 chip 下拉 ---');
  console.log(s2.slice(mi >= 0 ? mi : s2.indexOf('menu'), (mi >= 0 ? mi : s2.indexOf('menu')) + 800));
  await shot(page, 'a109_native_model_chip');
  await page.keyboard.press('Escape');
}, 's24b_native');
console.log('DONE');
