import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1200);
  // 聚焦 S1（左侧第一个会话）
  await page.getByText('未命名会话', { exact: true }).first().click();
  await sleep(800);
  await page.getByTestId('composer-input').locator('textarea').fill('hello');
  await shot(page, 'b24_before_send');
  await page.getByRole('button', { name: '发送' }).click();
  // 立即抓过渡态
  await sleep(700);
  await shot(page, 'b25_after_send_immediate');
  const snap1 = await snapshot(page);
  console.log('=== T+0.7s ===');
  console.log(snap1.split('\n').filter(l => /Queued|Working|Done|Failed|status|回合|发送|停止/.test(l)).join('\n'));
  // 等回合完成
  for (let i = 0; i < 20; i++) {
    await sleep(1000);
    const t = await page.locator('main').innerText();
    if (t.includes('Done') || t.includes('完成') || /hello world/i.test(t)) break;
  }
  await sleep(800);
  console.log('=== ARIA (after reply) ===');
  const snap2 = await snapshot(page);
  console.log(snap2);
  await shot(page, 'b26_acp_hello_reply');
}, 's28_acp_hello');
