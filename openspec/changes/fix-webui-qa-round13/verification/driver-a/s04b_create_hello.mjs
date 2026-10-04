import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

async function openWork(page) {
  await page.goto(BASE + '/');
  await sleep(1500);
}

await withPage(async (page) => {
  await openWork(page);
  // 建会话（claude, Ask）
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(1000);
  await page.getByRole('button', { name: '创建会话' }).click();
  // toast 瞬时态
  const toastAppeared = await page.getByText('已在「work」创建新会话').first().waitFor({ state: 'visible', timeout: 6000 }).then(() => true).catch(() => false);
  console.log('toast 创建会话: ' + toastAppeared);
  await shot(page, 'a09_session_created_toast');
  await sleep(1500);
  console.log((await snapshot(page)).slice(0, 2400));

  // 发 hello
  const composer = page.getByRole('textbox').last();
  await composer.fill('hello');
  await composer.press('Enter');
  await sleep(1000);
  await shot(page, 'a10_hello_sent');
  // 等回复
  await page.getByText('hello world').first().waitFor({ state: 'visible', timeout: 30000 }).then(() => console.log('hello world 出现')).catch(() => console.log('hello world 未出现'));
  await sleep(1000);
  const snap = await snapshot(page);
  console.log(snap.slice(0, 3200));
  await shot(page, 'a11_hello_done');
}, 's04b_hello');
console.log('DONE');
