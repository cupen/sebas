import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

const KEY = '/sessions/web%00web-1791068905608563800-0';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1200);

  // F5（本测试点本身就是刷新）
  await page.reload();
  await sleep(2500);
  const s1 = await snapshot(page);
  console.log('F5 后 URL: ' + page.url());
  console.log('F5 后聚焦会话: ' + (s1.includes('hello 🔒') ? 'hello（头部在）' : '未聚焦'));
  console.log('权限模式保持: ' + (s1.includes('combobox: Ask · 逐次询问') ? 'Ask 保持' : 'NOT Ask'));
  console.log('token 保持: ' + (s1.match(/Token in 1600 · out 160/)?.[0] || '未见'));
  console.log('消息保持: ' + (s1.includes('hello world') && s1.includes('parallel tools finished') && s1.includes('agent process exited or hung')));
  await shot(page, 'a26_f5_history_kept');

  // 头部徽章
  const badgeIdx = s1.indexOf('hello 🔒');
  console.log('头部: ' + s1.slice(badgeIdx, badgeIdx + 120));
}, 's09a_f5');
console.log('DONE');
