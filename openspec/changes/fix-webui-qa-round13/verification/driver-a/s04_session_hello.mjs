import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);

  // 新建会话
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(1200);
  console.log('--- 新会话对话框 ---');
  console.log((await snapshot(page)).slice(0, 2600));
  await shot(page, 'a08_new_session_dialog');
}, 's04a_dialog');
console.log('DONE');
