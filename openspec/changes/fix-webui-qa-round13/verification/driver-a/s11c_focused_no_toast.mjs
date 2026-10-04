import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';
import { execSync } from 'node:child_process';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';

const COOKIES = path.join(os.tmpdir(), 'qa13cookies.txt').replace(/\\/g, '/');
const BODY = path.join(os.tmpdir(), 'qa13msg.json').replace(/\\/g, '/');
fs.writeFileSync(BODY, JSON.stringify({ message: 'focused no-toast probe' }));
const CURL = `curl -s -b ${COOKIES} -X POST "http://127.0.0.1:9877/api/sessions/feishu%00agent-9511d900/message" -H "Content-Type: application/json" -d @${BODY}`;

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  // 聚焦 native 会话（侧栏 api hello native）
  await page.getByRole('listitem', { name: 'api hello native' }).first().click();
  await sleep(1500);
  const s0 = await snapshot(page);
  console.log('聚焦: ' + (s0.includes('api hello native') ? 'OK' : s0.slice(0, 400)));

  const seen = [];
  const watcher = (async () => {
    for (let i = 0; i < 80; i++) {
      const texts = await page.getByText(/回合已完成|的回合/).allTextContents().catch(() => []);
      for (const t of texts) { const tt = t.trim(); if (tt && !seen.includes(tt)) { seen.push(tt); console.log('TOAST-SEEN: ' + tt); } }
      await sleep(250);
    }
  })();
  await sleep(1000);
  console.log('curl: ' + execSync(CURL, { encoding: 'utf8' }).trim());
  await sleep(6000);
  console.log('聚焦态观察到的回合完成条: ' + JSON.stringify(seen) + '（空=不弹，符合既定语义）');
  await shot(page, 'a32_focused_no_toast');
}, 's11c_focused');
console.log('DONE');
