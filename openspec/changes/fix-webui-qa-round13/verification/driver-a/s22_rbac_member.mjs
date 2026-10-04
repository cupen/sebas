import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

async function login(page, u, p) {
  await page.locator('input[type="text"]').first().fill(u);
  await page.locator('input[type="password"]').first().fill(p);
  await page.getByRole('button', { name: '登录' }).click();
  await sleep(2500);
}
async function ensureLoginpage(page) {
  await page.goto(BASE + '/');
  await sleep(2500);
  const s = await snapshot(page);
  if (s.includes('退出登录')) {
    await page.getByRole('button', { name: '退出登录' }).click();
    await sleep(1800);
  }
}

await withPage(async (page) => {
  // 当前还是 qa-live → 拍设置 tabs 后登出
  await ensureLoginpage(page);
  // qa-live 仍在登录态？上轮脚本异常退出但 cookie 仍在
  const s0 = await snapshot(page);
  console.log('当前态: ' + (s0.includes('退出 (qa-live') ? 'qa-live' : s0.includes('退出登录') ? 'workbench其他' : '登录页'));
  if (s0.includes('退出 (qa-live')) {
    await page.getByRole('button', { name: '打开设置' }).click();
    await sleep(1200);
    await shot(page, 'a93_admin_settings_tabs');
    const d = await page.locator('[role="dialog"]').first().ariaSnapshot();
    const ni = d.indexOf('设置分区');
    console.log('qa-live(admin) tabs: ' + d.slice(ni, ni + 320).replace(/\n/g, ' '));
    // 技能 tab：admin 有无 🗑
    await page.getByText('技能', { exact: true }).first().click();
    await sleep(1000);
    const ds = await page.locator('[role="dialog"]').first().ariaSnapshot();
    console.log('qa-live(admin) 技能页: 删除按钮=' + (ds.includes('button "🗑"') ? '有' : '无') + '，同步按钮=' + (ds.includes('button "同步"') ? '有' : '无'));
    await shot(page, 'a94_admin_skills_tab');
    await page.keyboard.press('Escape');
    await sleep(500);
    await page.getByRole('button', { name: '退出登录' }).click();
    await sleep(1800);
  }

  // === member 横切 ===
  await login(page, 'member', 'member');
  const s1 = await snapshot(page);
  console.log('member 登录: ' + (s1.includes('退出 (member · member)') ? 'OK' : '失败'));
  console.log('member 侧栏: 添加项目按钮=' + (s1.includes('button "添加项目"') ? '有' : '无') + '，设置按钮=' + (s1.includes('button "打开设置"') ? '有' : '无'));
  await shot(page, 'a95_member_workbench');

  // member 建会话（work 项目行 + → 若可见）
  const newBtn = page.getByRole('button', { name: '在 work 中新建会话' });
  console.log('member 新建会话入口: ' + (await newBtn.count() > 0 ? '有' : '无'));
  // member 设置页
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  const dm = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const ni2 = dm.indexOf('设置分区');
  console.log('member tabs: ' + dm.slice(ni2, ni2 + 320).replace(/\n/g, ' '));
  console.log('member 用户 tab: ' + (dm.includes('button "用户"') ? '可见(异常?)' : '不可见'));
  await page.getByText('技能', { exact: true }).first().click();
  await sleep(1000);
  const dms = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('member 技能页: 删除按钮=' + (dms.includes('button "🗑"') ? '有(异常?)' : '无') + '，同步按钮=' + (dms.includes('button "同步"') ? '有(异常?R12-A-1回归!)' : '无（R12-A-1 保持）') + '，刷新=' + (dms.includes('button "刷新"') ? '有' : '无'));
  await shot(page, 'a96_member_skills_tab');
  await page.keyboard.press('Escape');
  await sleep(400);
  await page.getByRole('button', { name: '退出登录' }).click();
  await sleep(1800);

  // === viewer 横切 ===
  await login(page, 'viewer', 'viewer');
  const s2 = await snapshot(page);
  console.log('viewer 登录: ' + (s2.includes('退出 (viewer · viewer)') ? 'OK' : '失败'));
  console.log('viewer 侧栏: 添加项目=' + (s2.includes('button "添加项目"') ? '有' : '无') + '，新建会话=' + (s2.includes('button "在 work 中新建会话"') ? '有' : '无') + '，设置=' + (s2.includes('button "打开设置"') ? '有' : '无'));
  // composer 形态：聚焦会话后输入框是否禁用
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1500);
  const s3 = await snapshot(page);
  const tb = s3.match(/textbox:[^\n]*/);
  console.log('viewer 聚焦会话 composer: ' + (tb ? tb[0] : '无 textbox'));
  console.log('viewer 发送按钮: ' + (s3.includes('button "发送"') ? '有' : '无'));
  await shot(page, 'a97_viewer_session_view');
  // viewer 设置页
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  const dv = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const ni3 = dv.indexOf('设置分区');
  console.log('viewer tabs: ' + (ni3 >= 0 ? dv.slice(ni3, ni3 + 320).replace(/\n/g, ' ') : dv.slice(0, 300).replace(/\n/g, ' ')));
  await shot(page, 'a98_viewer_settings');
}, 's22_rbac');
console.log('DONE');
