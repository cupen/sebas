import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  const s0 = await snapshot(page);
  if (!s0.includes('退出 (viewer')) {
    if (s0.includes('退出登录')) {
      await page.getByRole('button', { name: '退出登录' }).click();
      await sleep(1800);
    }
    await page.locator('input[type="text"]').first().fill('viewer');
    await page.locator('input[type="password"]').first().fill('viewer');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2500);
  }
  console.log('viewer 态: ' + ((await snapshot(page)).match(/退出 \(([^\)]+)\)/)?.[1] || '?') );

  // 1) composer 禁用态：聚焦 hello 会话
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1500);
  const s1 = await snapshot(page);
  const tbIdx = s1.indexOf('- textbox');
  console.log('composer aria: ' + s1.slice(tbIdx, tbIdx + 120).replace(/\n/g, ' | '));
  const sendDisabled = await page.getByRole('button', { name: '发送' }).isDisabled().catch(() => '无按钮');
  console.log('发送按钮 disabled: ' + sendDisabled);
  const comboDisabled = await page.getByRole('combobox').last().getAttribute('aria-disabled').catch(() => null);
  console.log('权限 combobox aria-disabled: ' + comboDisabled);
  // 尝试输入（应被 readOnly/disabled 拦截）
  const tb = page.locator('textarea').first();
  const readOnly = await tb.getAttribute('readonly').catch(() => null);
  const dis = await tb.getAttribute('disabled').catch(() => null);
  console.log('textarea readonly=' + readOnly + ' disabled=' + dis);
  await shot(page, 'a99_viewer_composer');

  // 2) viewer 技能页 删除/同步
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('技能', { exact: true }).first().click();
  await sleep(1000);
  const ds = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('viewer 技能页: 删除=' + (ds.includes('button "🗑"') ? '有(异常!)' : '无') + '，同步=' + (ds.includes('button "同步"') ? '有(异常!R12-A-1)' : '无（R12-A-1 保持）'));
  await shot(page, 'a100_viewer_skills');
  await page.keyboard.press('Escape');
  await sleep(400);

  // 3) viewer 点「添加项目」会怎样
  const addBtn = page.getByRole('button', { name: '添加项目' });
  console.log('viewer 添加项目按钮数: ' + await addBtn.count());
  if (await addBtn.count() > 0) {
    await addBtn.click();
    await sleep(1200);
    const s2 = await snapshot(page);
    console.log('viewer 添加项目对话框: ' + (s2.includes('dialog') && s2.includes('选择要添加为项目的目录') ? '打开了' : '未打开'));
    console.log('错误提示: ' + (s2.match(/alert:?[^\n]*/)?.[0] || '无'));
    await shot(page, 'a101_viewer_addproject');
    // 若打开了就取消
    const cancel = page.getByRole('button', { name: '取消' });
    if (await cancel.count() > 0) { await cancel.click(); await sleep(600); }
  }
}, 's22b_viewer');
console.log('DONE');
