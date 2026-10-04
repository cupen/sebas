import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1500);

  const s0 = await snapshot(page);
  const before = s0.match(/Token in (\d+) · out (\d+)/);
  console.log('恢复前 token: ' + before?.[0] + '；含模型已切换卡: ' + s0.includes('模型已切换'));

  // 发后续消息触发恢复
  const composer = page.getByRole('textbox').last();
  await composer.fill('hello again');
  await composer.press('Enter');
  let ok = false;
  for (let i = 0; i < 25; i++) {
    await sleep(2000);
    const s = await snapshot(page);
    if (s.includes('hello world')) { ok = true; break; }
  }
  console.log('恢复回合完成: ' + ok);
  await sleep(1500);
  const s2 = await snapshot(page);
  const after = s2.match(/Token in (\d+) · out (\d+)/);
  console.log('恢复后 token: ' + after?.[0]);
  console.log('含「模型已切换」卡: ' + s2.includes('模型已切换'));
  const ci = s2.indexOf('paragraph: crash');
  console.log('--- crash→恢复 段落 ---');
  console.log(s2.slice(ci, ci + 1800));
  await shot(page, 'a25_crash_recovered');
}, 's08b_recover');
console.log('DONE');
