import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';
import { execSync } from 'node:child_process';
import path from 'node:path';
import os from 'node:os';

const COOKIES = path.join(os.tmpdir(), 'qa13cookies.txt').replace(/\\/g, '/');
const CURL = `curl -s -b ${COOKIES} -X POST "http://127.0.0.1:9877/api/sessions/feishu%00agent-9511d900/message" -H "Content-Type: application/json" -d '{"message":"toast probe 1"}'`;

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1200);
  // GUI 点击历史链接
  await page.getByRole('link', { name: '历史' }).click();
  await sleep(1500);
  console.log('URL: ' + page.url());
  await shot(page, 'a29_history_page');

  // 武装 toast 观察器
  const seen = [];
  const watcher = (async () => {
    for (let i = 0; i < 240; i++) {  // 60s
      const texts = await page.locator('[class*="toast" i], [role="status"], [role="alert"], [class*="notification" i]').allTextContents().catch(() => []);
      for (const t of texts) {
        const tt = t.trim();
        if (tt && !seen.includes(tt)) { seen.push(tt); console.log('TOAST: ' + JSON.stringify(tt)); }
      }
      await sleep(250);
    }
  })();

  // 等 2s 确认基线无 toast，然后 API 触发回合
  await sleep(2000);
  const out = execSync(CURL, { encoding: 'utf8' });
  console.log('curl 发消息: ' + out.trim());
  await sleep(4000);
  await shot(page, 'a30_r12b2_toast_window');
  await sleep(3000);
  console.log('观察到的 toast: ' + JSON.stringify(seen));
  // 停止 watcher（循环自然结束或直接放弃）
}, 's11_r12b2');
console.log('DONE');
