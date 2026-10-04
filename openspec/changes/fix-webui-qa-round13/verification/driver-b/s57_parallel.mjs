import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

async function focusAgent(page, agentRe, nameRe) {
  const rows = page.getByRole('listitem', { name: /.+/ });
  const n = await rows.count();
  for (let i = 0; i < n; i++) {
    const row = rows.nth(i);
    const nm = (await row.textContent().then(t => (t || '').trim().slice(0, 24))) || '';
    if (/work/.test(nm)) continue;
    await row.click();
    await sleep(900);
    const s = await snapshot(page);
    if (agentRe.test(s) && (!nameRe || nameRe.test(s))) { console.log('matched row[' + i + '] ' + nm); return true; }
  }
  return false;
}

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1500);
  const snap0 = await snapshot(page);
  if (snap0.includes('登录以继续')) {
    await page.getByRole('textbox', { name: '用户名' }).fill('admin');
    await page.getByRole('textbox', { name: '密码' }).fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2000);
  }
  // claude 会话现在叫 drip（自动命名）
  if (!(await focusAgent(page, /🔒 claude /))) throw new Error('claude not found');
  // 确认 Ask 档（新会话默认 Ask；此会话之前是 Ask）
  await page.getByTestId('composer-input').locator('textarea').fill('parallel');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(2000);
  await shot(page, 'b63_parallel_cards');
  const s = await snapshot(page);
  console.log('=== parallel cards ===');
  console.log(s.split('\n').filter(l => /region|仅允许|拒绝|本会话|等待|审批|过程|Bash/.test(l)).slice(0, 24).join('\n'));
}, 's57_parallel');
