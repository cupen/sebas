import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  const f404 = [];
  page.on('response', (r) => { if (r.status() >= 400) f404.push({ t: Date.now(), s: r.status(), u: r.url() }); });

  // 死会话深链
  await page.goto(BASE + '/sessions/web%00web-DEAD-0000-999');
  await sleep(3000);
  console.log('死链首载 3s 内 4xx 数: ' + f404.length);
  console.log('页面可见文本头部: ' + (await snapshot(page)).slice(0, 600).replace(/\n/g, ' | '));
  await shot(page, 'a27_deadlink_first_load');

  // 静置 12s 再数——是否持续增长
  const c1 = f404.length;
  await sleep(12000);
  console.log('静置 12s 后 4xx 数: ' + f404.length + '（增量 ' + (f404.length - c1) + '）');

  // 轻交互（点导航）后是否继续刷
  await page.getByRole('link', { name: '用量统计' }).click().catch(() => {});
  await sleep(3000);
  console.log('交互后 4xx 总数: ' + f404.length);
  console.log('4xx 明细: ' + JSON.stringify(f404.slice(0, 8)));
  await shot(page, 'a28_deadlink_after_idle');
}, 's10_o5_deadlink');
console.log('DONE');
