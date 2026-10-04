import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(1000);
  // Agent 下拉选 claude-think
  const agentCombo = page.getByRole('combobox').first();
  await agentCombo.click();
  await sleep(600);
  console.log('--- Agent 下拉选项 ---');
  const s0 = await snapshot(page);
  const li = s0.indexOf('listbox');
  console.log(s0.slice(li, li + 700));
  await shot(page, 'a52_agent_dropdown');
  await page.getByRole('option', { name: /claude-think/ }).click();
  await sleep(600);
  await page.getByRole('button', { name: '创建会话' }).click();
  await sleep(1500);

  // 发消息触发 thinking 场景
  const composer = page.getByRole('textbox').first();
  await composer.fill('想一下再答');
  await composer.press('Enter');
  // 等过程 chip 出现
  await page.getByText(/过程/).first().waitFor({ state: 'visible', timeout: 30000 }).catch(() => {});
  await sleep(2500);
  const s1 = await snapshot(page);
  const gi = s1.indexOf('log "会话对话"');
  console.log('--- thinking 会话回合 ---');
  console.log(s1.slice(gi, gi + 1600));
  await shot(page, 'a53_think_session_turn');

  // 展开 thinking chip（B-2 复采）
  const chip = page.getByRole('button', { name: /过程 thinking/ }).first();
  if (await chip.count() > 0) {
    await chip.click();
    await sleep(1000);
    await shot(page, 'a54_thinking_expanded_b2_recheck');
    console.log('B-2 复采截图已拍');
  } else {
    console.log('无 thinking chip');
  }
}, 's15d_think');
console.log('DONE');
