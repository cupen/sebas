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
  const modelCombo = page.getByRole('combobox').nth(2);
  await modelCombo.click();
  await sleep(600);
  await page.getByRole('option', { name: 'test/long' }).click();
  await sleep(500);
  await page.getByRole('button', { name: '创建会话' }).click();
  await sleep(2200);

  const tb = page.locator('textarea').first();
  await tb.fill('流式长文');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(1200);
  // 流式中途：截中间帧
  const s1 = await snapshot(page);
  const stopVisible = s1.includes('button "停止回复"');
  console.log('流式中停止按钮: ' + stopVisible);
  await shot(page, 'a118_long_streaming_mid');

  // T+1.5s 点停止
  await page.getByRole('button', { name: '停止回复' }).click();
  await sleep(1000);
  await shot(page, 'a119_long_stop_clicked');
  await sleep(3000);
  const s2 = await snapshot(page);
  const gi = s2.indexOf('paragraph: 流式长文');
  console.log('--- 停止后 ---');
  console.log(s2.slice(gi, gi + 1000));
  console.log('取消提示: ' + (s2.includes('取消') ? '有取消卡' : '无'));
  await shot(page, 'a120_long_stopped');

  // 跟发消息确认会话可用
  await tb.fill('还能继续吗');
  await page.getByRole('button', { name: '发送' }).click();
  const ok = await page.getByText(/test provider/).first().waitFor({ state: 'visible', timeout: 25000 }).then(() => true).catch(() => false);
  console.log('停止后跟发回合: ' + ok);
  await shot(page, 'a121_long_after_followup');
}, 's25e_long');
console.log('DONE');
