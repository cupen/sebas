import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1200);

  const before = (await snapshot(page)).match(/Token in (\d+) · out (\d+)/);
  console.log('crash 前 token: ' + before?.[0]);

  // 发 crash
  const composer = page.getByRole('textbox').last();
  await composer.fill('crash');
  await composer.press('Enter');
  await sleep(2500);
  await shot(page, 'a23_crash_sent');

  // 轮询 45s 看恢复形态
  let observed = '';
  for (let i = 0; i < 22; i++) {
    await sleep(2000);
    const s = await snapshot(page);
    if (/Token in (\d+) · out (\d+)/.test(s)) {
      const m = s.match(/Token in (\d+) · out (\d+)/);
      if (Number(m[1]) > Number(before[1])) { observed = 'token续增 ' + m[0]; break; }
      if (s.includes('错误') || s.includes('崩溃') || s.includes('已切换')) { observed = '错误/回执卡出现'; break; }
    }
  }
  console.log('轮询观察: ' + (observed || '45s 内无 token 变化'));
  const s2 = await snapshot(page);
  const li = s2.indexOf('paragraph: crash');
  console.log('--- crash 回合后 ---');
  console.log(s2.slice(li, li + 1400));
  await shot(page, 'a24_crash_after');
}, 's08a_crash');
console.log('DONE');
