import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  // 回到 tool-use 会话（最后一个未命名会话/最新）
  await page.getByText('跑一个工具环', { exact: false }).first().click().catch(() => {});
  await sleep(1500);
  const s0 = await snapshot(page);
  if (!s0.includes('权限审批')) {
    // 找 tool-use 会话：侧栏找含 tool 的会话名
    console.log('审批区不在，列出侧栏会话');
    const li = s0.indexOf('项目');
    console.log(s0.slice(li, li + 700));
  }
  await page.getByRole('button', { name: '仅允许一次' }).click();
  await sleep(2500);
  await shot(page, 'a116_tooluse_approved');
  // 等终文本
  for (let i = 0; i < 10; i++) {
    await sleep(1500);
    const s = await snapshot(page);
    if (s.includes('权限审批') === false) break;
  }
  const s1 = await snapshot(page);
  const gi = s1.indexOf('paragraph: 跑一个工具环');
  console.log('--- 批准后 ---');
  console.log(s1.slice(gi, gi + 1200));
  await shot(page, 'a117_tooluse_done');
}, 's25d_approve');
console.log('DONE');
