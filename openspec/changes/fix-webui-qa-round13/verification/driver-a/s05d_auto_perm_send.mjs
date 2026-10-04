import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1200);

  const composer = page.getByRole('textbox').last();
  await composer.fill('perm');
  await composer.press('Enter');
  await sleep(1000);
  await shot(page, 'a14_auto_perm_sent');
  // 等回合终结：头部状态或 token 变化；轮询快照最多 30s
  let done = false;
  for (let i = 0; i < 15; i++) {
    await sleep(2000);
    const s = await snapshot(page);
    if (s.includes('✓') || s.includes('已执行') || /Token in \d+ · out \d+/.test(s)) {
      const m = s.match(/Token in \d+ · out \d+/);
      if (m && i >= 2) { done = true; break; }
    }
  }
  console.log('轮询结束 done=' + done);
  const s2 = await snapshot(page);
  console.log('--- perm 回合后 ---');
  console.log(s2.slice(s2.indexOf('log "会话对话"'), s2.indexOf('log "会话对话"') + 2200));
  await shot(page, 'a15_auto_perm_done');
}, 's05d_auto_perm');
console.log('DONE');
