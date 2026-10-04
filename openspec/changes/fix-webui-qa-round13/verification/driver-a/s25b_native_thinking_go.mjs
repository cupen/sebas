import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  const agentCombo = page.getByRole('combobox').first();
  await agentCombo.click();
  await sleep(500);
  await page.getByRole('option', { name: /Native Kernel/ }).click();
  await sleep(700);
  // 对话框默认 Provider=fake13 / 模型=目录第一项（test/thinking）——直接创建
  await page.getByRole('button', { name: '创建会话' }).click();
  await sleep(2200);

  const tb = page.locator('textarea').first();
  await tb.fill('深想一下');
  await page.getByRole('button', { name: '发送' }).click();
  await page.getByText(/thinking/i).first().waitFor({ state: 'visible', timeout: 30000 }).catch(() => {});
  await sleep(2500);
  const s1 = await snapshot(page);
  const gi = s1.indexOf('log "会话对话"');
  console.log('--- native test/thinking 回合 ---');
  console.log(s1.slice(gi, gi + 1200));
  await shot(page, 'a114_native_thinking');
}, 's25b_go');
console.log('DONE');
