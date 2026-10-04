import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';
import { execSync } from 'node:child_process';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';

const COOKIES = path.join(os.tmpdir(), 'qa13cookies.txt').replace(/\\/g, '/');
const BODY = path.join(os.tmpdir(), 'qa13msg.json').replace(/\\/g, '/');
fs.writeFileSync(BODY, JSON.stringify({ message: 'toast probe 2' }));
const CURL = `curl -s -b ${COOKIES} -X POST "http://127.0.0.1:9877/api/sessions/feishu%00agent-9511d900/message" -H "Content-Type: application/json" -d @${BODY}`;

await withPage(async (page) => {
  await page.goto(BASE + '/sessions');
  await sleep(1800);
  console.log('URL: ' + page.url());

  const seen = [];
  const watcher = (async () => {
    for (let i = 0; i < 160; i++) {  // 40s
      const texts = await page.locator('[class*="toast" i], [class*="notification" i], [class*="notice" i]').allTextContents().catch(() => []);
      for (const t of texts) {
        const tt = t.trim();
        if (tt && !seen.includes(tt) && tt !== '核心已连接') { seen.push(tt); console.log('TOAST: ' + JSON.stringify(tt)); }
      }
      await sleep(250);
    }
  })();

  await sleep(1500);
  const out = execSync(CURL, { encoding: 'utf8' });
  console.log('curl 发消息: ' + out.trim());
  await sleep(5000);
  await shot(page, 'a31_r12b2_toast_window2');
  await sleep(2000);
  console.log('观察到的 toast（排除常驻连接条）: ' + JSON.stringify(seen));
}, 's11b_r12b2');
console.log('DONE');
