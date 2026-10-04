import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  const s0 = await snapshot(page);
  if (!s0.includes('退出 (viewer')) {
    if (s0.includes('退出登录')) { await page.getByRole('button', { name: '退出登录' }).click(); await sleep(1800); }
    await page.locator('input[type="text"]').first().fill('viewer');
    await page.locator('input[type="password"]').first().fill('viewer');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2500);
  }

  // 1) viewer 尝试打字并发送
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1500);
  const tb = page.locator('textarea').first();
  await tb.fill('viewer write probe');
  await sleep(600);
  const sendBtn = page.getByRole('button', { name: '发送' });
  const enabled = await sendBtn.isEnabled().catch(() => false);
  console.log('输入后发送按钮 enabled: ' + enabled);
  await shot(page, 'a102_viewer_typed');
  if (enabled) {
    await sendBtn.click();
    await sleep(3000);
    const s1 = await snapshot(page);
    console.log('viewer 消息发出后: ' + (s1.includes('viewer write probe') ? '消息出现在转录(异常!)' : '未出现'));
    console.log('错误提示: ' + (s1.match(/alert:?[^\n]*/)?.[0] || (s1.includes('错误') ? '有错误卡' : '无')));
    await shot(page, 'a103_viewer_send_result');
  }

  // 2) viewer 尝试注册项目
  await page.getByRole('button', { name: '添加项目' }).click();
  await sleep(1200);
  await page.getByRole('treeitem', { name: 'downloads' }).click();
  await sleep(2500);
  await page.getByRole('button', { name: '添加项目' }).last().click();
  await sleep(2000);
  const s2 = await snapshot(page);
  console.log('viewer 注册 downloads: ' + (s2.includes('downloads local') ? '成功(异常!越权)' : '未成功'));
  console.log('提示: ' + (s2.match(/alert:?[^\n]*/)?.[0] || (s2.includes('toast') || s2.includes('权限') ? '见截图' : '无')));
  await shot(page, 'a104_viewer_register_project');
}, 's22c_viewer_write');
console.log('DONE');
