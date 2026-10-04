import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1500);

  const s0 = await snapshot(page);
  if (!s0.includes('权限审批')) { console.log('审批区不在——卡可能已消失'); console.log(s0.slice(-1500)); }
  console.log('审批卡仍在: ' + s0.includes('仅允许一次'));

  // 第一张卡（Bash）批准
  await page.getByRole('button', { name: '仅允许一次' }).first().click();
  await sleep(1500);
  await shot(page, 'a20_parallel_first_approved');

  // 第二张卡（Read）拒绝
  const rej = page.getByRole('button', { name: '拒绝' });
  console.log('剩余拒绝按钮: ' + await rej.count());
  await rej.first().click();
  await sleep(1200);
  await shot(page, 'a21_parallel_second_rejected');

  // 等回合终结
  for (let i = 0; i < 12; i++) {
    await sleep(2000);
    const s = await snapshot(page);
    if (!s.includes('权限审批')) break;
  }
  await sleep(1000);
  const s2 = await snapshot(page);
  const tail = s2.lastIndexOf('parallel');
  console.log('--- 双卡裁决后尾部 ---');
  console.log(s2.slice(s2.indexOf('权限模式已切换：逐次询问'), s2.indexOf('权限模式已切换：逐次询问') + 2400));
  await shot(page, 'a22_parallel_final_states');
}, 's07b_decide');
console.log('DONE');
