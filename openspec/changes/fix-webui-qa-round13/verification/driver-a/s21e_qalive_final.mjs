import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

const userInput = () => page.locator('input[type="text"]').first();
let page;

async function login(page, u, p) {
  await page.locator('input[type="text"]').first().fill(u);
  await page.locator('input[type="password"]').first().fill(p);
  await page.getByRole('button', { name: '登录' }).click();
  await sleep(2500);
}

await withPage(async (pg) => {
  page = pg;
  await page.goto(BASE + '/');
  await sleep(2500);

  // 若已在工作台则先登出
  const s0 = await snapshot(page);
  if (s0.includes('退出登录')) {
    console.log('已在工作台，先登出');
    await page.getByRole('button', { name: '退出登录' }).click();
    await sleep(1800);
  }
  await login(page, 'admin', 'admin');
  const s1 = await snapshot(page);
  console.log('admin 登录: ' + (s1.includes('退出 (admin · root)') ? 'OK' : '失败'));

  // 用户 tab 检查 qa-live 状态
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('用户', { exact: true }).first().click();
  await sleep(1000);
  const d = await page.locator('[role="dialog"]').first().ariaSnapshot();
  const qi = d.indexOf('qa-live');
  console.log('qa-live 状态行: ' + d.slice(qi, qi + 220).replace(/\n/g, ' | '));
  await shot(page, 'a87_qalive_state');
  // 若禁用则启用
  if (d.slice(qi, qi + 220).includes('已禁用')) {
    await page.getByRole('button', { name: '■' }).last().click();
    await sleep(1500);
    const d2 = await page.locator('[role="dialog"]').first().ariaSnapshot();
    console.log('启用操作后 qa-live: ' + d2.slice(d2.indexOf('qa-live'), d2.indexOf('qa-live') + 160).replace(/\n/g, ' | '));
  }
  await page.keyboard.press('Escape');
  await sleep(600);
  // 登出 → qa-live 登录
  await page.getByRole('button', { name: '退出登录' }).click();
  await sleep(1800);
  await login(page, 'qa-live', 'qa-live-2026');
  const s2 = await snapshot(page);
  if (s2.includes('退出 (qa-live')) {
    console.log('qa-live 登录成功: ' + s2.match(/退出 \(qa-live[^\)]*\)/)?.[0]);
    await shot(page, 'a89_qalive_logged_in');
  } else {
    console.log('qa-live 登录仍失败: ' + (s2.match(/alert: [^\n]+/)?.[0] || s2.slice(0, 150)));
    await shot(page, 'a89_qalive_login_failed');
  }
}, 's21e_final');
console.log('DONE');
