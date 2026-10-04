import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

async function newNativeSession(page, modelOption) {
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  const agentCombo = page.getByRole('combobox').first();
  await agentCombo.click();
  await sleep(500);
  await page.getByRole('option', { name: /Native Kernel/ }).click();
  await sleep(700);
  // 现在应有 Provider/模型 下拉
  const d = await snapshot(page);
  const di = d.indexOf('dialog');
  console.log('--- 对话框 ---');
  console.log(d.slice(di, di + 1300));
  if (modelOption) {
    // 最后一个 combobox 是模型（Agent 之后）
    const combos = page.getByRole('combobox');
    const cn = await combos.count();
    console.log('combobox 数: ' + cn);
    const modelCombo = combos.nth(cn - 2); // 倒数第二=模型，最后=权限模式
    await modelCombo.click();
    await sleep(600);
    await page.getByRole('option', { name: modelOption }).click();
    await sleep(500);
  }
  await page.getByRole('button', { name: '创建会话' }).click();
  await sleep(2000);
}

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  await newNativeSession(page, 'test/thinking');

  const tb = page.locator('textarea').first();
  await tb.fill('深想一下');
  await page.getByRole('button', { name: '发送' }).click();
  await page.getByText(/过程|thinking/).first().waitFor({ state: 'visible', timeout: 30000 }).catch(() => {});
  await sleep(2500);
  const s1 = await snapshot(page);
  const gi = s1.indexOf('log "会话对话"');
  console.log('--- native thinking 回合 ---');
  console.log(s1.slice(gi, gi + 1100));
  await shot(page, 'a114_native_thinking');
}, 's25_native_thinking');
console.log('DONE');
