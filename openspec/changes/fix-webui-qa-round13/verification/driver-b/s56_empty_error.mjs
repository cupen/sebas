import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

async function focusAgent(page, agentRe) {
  const rows = page.getByRole('listitem', { name: /.+/ });
  const n = await rows.count();
  for (let i = 0; i < n; i++) {
    const row = rows.nth(i);
    const nm = (await row.textContent().then(t => (t || '').trim().slice(0, 24))) || '';
    if (/work/.test(nm)) continue;
    await row.click();
    await sleep(900);
    if (agentRe.test(await snapshot(page))) { console.log('matched row[' + i + '] ' + nm); return true; }
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
  // B.7 empty
  if (!(await focusAgent(page, /🔒 empty /))) throw new Error('empty not found');
  await page.getByTestId('composer-input').locator('textarea').fill('给我一个空回合');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(3000);
  await shot(page, 'b60_empty_turn');
  const s1 = await snapshot(page);
  console.log('=== empty turn ===');
  console.log(s1.split('\n').filter(l => /paragraph|Empty|空|Done|text:.*token|Token/.test(l)).slice(0, 14).join('\n'));

  // B.8 error
  if (!(await focusAgent(page, /🔒 error /))) throw new Error('error not found');
  await page.getByTestId('composer-input').locator('textarea').fill('触发一个错误');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(4000);
  await shot(page, 'b61_error_turn');
  const s2 = await snapshot(page);
  console.log('=== error turn ===');
  console.log(s2.split('\n').filter(l => /paragraph|错误|error|失败|Failed|重试|Token/.test(l)).slice(0, 16).join('\n'));
  // 会话仍可用？
  await page.getByTestId('composer-input').locator('textarea').fill('错误后还能发吗');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(4000);
  await shot(page, 'b62_error_session_reuse');
  const s3 = await snapshot(page);
  console.log('=== error session reuse ===');
  console.log(s3.split('\n').filter(l => /paragraph|错误|失败|Failed/.test(l)).slice(0, 16).join('\n'));
}, 's56_empty_error');
