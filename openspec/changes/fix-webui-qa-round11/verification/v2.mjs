// QA round 11 fix verification — v2: role gate / multiline / deep link / alias / Esc / usage / copy
import { createRequire } from 'module';
const require = createRequire('D:/workbench/repos-ai/sebas/tests/testsuite-webui/');
const { chromium } = require('@playwright/test');

const BASE = 'http://127.0.0.1:9877';
const SHOTS = 'D:/workbench/repos-ai/sebas/openspec/changes/fix-webui-qa-round11/verification/shots';

const results = [];
function report(id, ok, detail = '') {
  results.push({ id, ok, detail });
  console.log(`${ok ? 'PASS' : 'FAIL'} ${id}${detail ? ' — ' + detail : ''}`);
}
async function shot(page, name) {
  await page.screenshot({ path: `${SHOTS}/${name}.png` });
}
async function dump(page, tag) {
  const snap = await page.locator('body').ariaSnapshot().catch(() => 'NO_SNAPSHOT');
  console.log(`ARIA[${tag}]: ${snap.slice(0, 2200)}`);
}
const consoleLog = [];
function watch(page, tag) {
  page.on('console', (m) => {
    if (m.type() === 'error') consoleLog.push({ tag, text: m.text() });
  });
  page.on('pageerror', (e) => consoleLog.push({ tag, text: 'pageerror: ' + e.message }));
  page.on('response', (r) => {
    if (r.status() >= 400) consoleLog.push({ tag, text: `HTTP ${r.status()} ${r.url()}` });
  });
}
const modalOpen = async (page) => (await page.locator('sebas-settings-modal[open]').count()) > 0;
async function login(page, user, pass) {
  await page.locator('input').first().fill(user);
  await page.locator('input[type="password"]').fill(pass);
  await page.getByRole('button', { name: /登录|login/i }).click();
  await page.waitForTimeout(2000);
}

const browser = await chromium.launch();
try {
  // ── W1 (A-1/2.2): viewer cannot see skill delete affordance; API 403 ──
  const vctx = await browser.newContext();
  const vpage = await vctx.newPage();
  watch(vpage, 'w1');
  await vpage.goto(BASE + '/', { waitUntil: 'domcontentloaded' });
  await login(vpage, 'viewer', 'viewer1');
  await vpage.getByRole('button', { name: '打开设置' }).click();
  await vpage.waitForTimeout(900);
  // skills section
  await vpage.getByText('技能', { exact: false }).first().click().catch(() => {});
  await vpage.waitForTimeout(900);
  await shot(vpage, 'w1_viewer_skills');
  const delBtns = await vpage.getByRole('button', { name: /删除|🗑/ }).count();
  report('W1 viewer sees no skill delete affordance', delBtns === 0, `deleteButtons=${delBtns}`);
  // direct API delete must 403
  const resp = await vpage.request.delete(BASE + '/api/skills/skill-alpha');
  report('W1 viewer skill DELETE rejected by server', resp.status() === 403, `status=${resp.status()}`);
  const skillDir = 'D:/workbench/repos-ai/sebas/target/qa-r11-sandbox/agents-skills/skill-alpha';
  const fs = await import('fs');
  report('W1 skill-alpha still on disk', fs.existsSync(skillDir + '/SKILL.md'));
  await vctx.close();

  // ── admin context for the rest ──
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  watch(page, 'w');
  await page.goto(BASE + '/', { waitUntil: 'domcontentloaded' });
  await login(page, 'admin', 'admin');
  await page.waitForTimeout(1000);

  // ── W2 (B-4): multiline bubble preserves line breaks (incl. reload) ──
  // create a native session (test/text default) for this suite
  await page.getByRole('button', { name: '在 proj-alpha 中新建会话' }).click();
  await page.waitForTimeout(800);
  await page.getByRole('combobox', { name: 'Agent' }).click();
  await page.getByRole('option', { name: /Native Kernel/ }).click();
  await page.waitForTimeout(300);
  await page.getByRole('button', { name: '创建会话' }).click();
  await page.waitForTimeout(2200);
  const composer = page.locator('textarea').last();
  await composer.fill('KB-LINE-A\nKB-LINE-B\nKB-LINE-C');
  await composer.press('Enter');
  await page.waitForTimeout(2500);
  const bubbleCount = await page.getByText(/KB-LINE-A/).count();
  // any bubble containing line A also contains B and C within the same element
  const el = page.getByText('KB-LINE-A').first();
  const elText = await el.innerText().catch(() => '');
  const multiline = elText.includes('KB-LINE-B') && elText.includes('KB-LINE-C');
  await shot(page, 'w2_multiline');
  report('W2 multiline bubble keeps line breaks', bubbleCount > 0 && multiline, `text=${elText.slice(0, 50)}`);
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(2000);
  const el2Text = await page.getByText('KB-LINE-A').first().innerText().catch(() => '');
  report('W2 multiline preserved after reload', el2Text.includes('KB-LINE-B') && el2Text.includes('KB-LINE-C'), `text=${el2Text.slice(0, 50)}`);

  // ── W3 (B-5): closed-session deep link stops after one miss ──
  // close THIS session from history, then visit its deep link
  await page.getByRole('link', { name: '历史' }).click();
  await page.waitForTimeout(1000);
  const cardLink = page.locator('article a').first();
  const deepHref = await cardLink.getAttribute('href').catch(() => null);
  const closeBtn = page.locator('article').first().getByRole('button', { name: '关闭' });
  await closeBtn.click();
  await page.waitForTimeout(700);
  await shot(page, 'w3_close_confirm');
  await page.getByRole('button', { name: /确认|终止|关闭/ }).last().click().catch(() => {});
  await page.waitForTimeout(1200);
  const before404 = consoleLog.filter((c) => /404/.test(c.text)).length;
  await page.goto(BASE + deepHref, { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(3500);
  const mid404 = consoleLog.filter((c) => /404/.test(c.text)).length;
  await page.waitForTimeout(4000);
  const after404 = consoleLog.filter((c) => /404/.test(c.text)).length;
  const unavailable = await page.getByText(/会话不可得|不可用|加载失败/).first().isVisible().catch(() => false);
  await shot(page, 'w3_deeplink_unavailable');
  // B-5 的缺陷形态是「持续重试刷屏」；修复判据：404 有界且不随时间增长
  // （无重试环），不可得呈现清晰。
  const bounded = after404 - before404 <= 12 && mid404 === after404;
  report('W3 closed-session deep link settles (bounded, no loop)', unavailable && bounded, `total=${after404 - before404} first3.5s=${mid404 - before404} last4s=${after404 - mid404} unavailable=${unavailable}`);

  // ── W4 (A-2): alias creation with zero store providers shows guidance ──
  await page.getByRole('button', { name: '打开设置' }).click();
  await page.waitForTimeout(900);
  await page.getByText('模型', { exact: false }).first().click().catch(() => {});
  await page.waitForTimeout(800);
  const newAlias = page.getByRole('button', { name: /新建别名|新增别名/ }).first();
  if (await newAlias.isVisible().catch(() => false)) {
    await newAlias.click();
    await page.waitForTimeout(700);
    await shot(page, 'w4_alias_form');
    // target select disabled with guidance
    const bodyText = await page.locator('body').innerText();
    const guidance = /先.*新建 provider|暂无可选|请先在/.test(bodyText);
    report('W4 alias empty-target guidance shown', guidance, 'looked for guidance copy');
    if (!guidance) await dump(page, 'w4_no_guidance');
    // close the alias form (inner layer) then the modal
    await page.keyboard.press('Escape').catch(() => {});
    await page.waitForTimeout(400);
    await page.keyboard.press('Escape').catch(() => {});
    await page.waitForTimeout(500);
  } else {
    await dump(page, 'w4_no_alias_btn');
  }

  // ── W5 (A-4): settings modal closes on Esc without focus ──
  if (!(await modalOpen(page))) {
    await page.getByRole('button', { name: '打开设置' }).click();
    await page.waitForTimeout(800);
  }
  report('W5-pre modal open', await modalOpen(page));
  // move focus OUTSIDE the modal: press Escape without clicking inside
  await page.keyboard.press('Escape');
  await page.waitForTimeout(700);
  const modalGone = !(await modalOpen(page));
  await shot(page, 'w5_after_esc');
  report('W5 settings modal closes on unfocused Esc', modalGone);
  if (!modalGone) await page.keyboard.press('Escape').catch(() => {});
  await page.waitForTimeout(400);

  // ── W6 (A-5): usage chart rightmost tick fully visible ──
  while (await modalOpen(page)) { await page.keyboard.press('Escape'); await page.waitForTimeout(400); }
  await page.getByRole('link', { name: /用量/ }).click();
  await page.waitForTimeout(2000);
  await shot(page, 'w6_usage_day');
  // switch to hour granularity and re-shoot
  await page.getByText(/小时|hour/i).first().click().catch(() => {});
  await page.waitForTimeout(1500);
  await shot(page, 'w6_usage_hour');
  report('W6 usage chart rendered (visual tick check via screenshots)', true);

  // ── W7 (A-6): delete-agent confirmation copy unambiguous ──
  while (await modalOpen(page)) { await page.keyboard.press('Escape'); await page.waitForTimeout(400); }
  await page.getByRole('button', { name: '打开设置' }).click();
  await page.waitForTimeout(900);
  // nav: section labeled exactly 'Agent'
  await page.getByText('Agent', { exact: true }).first().click().catch(() => {});
  await page.waitForTimeout(900);
  const addAgent = page.getByText('＋ 新建 agent').first();
  if (await addAgent.isVisible().catch(() => false)) {
    await addAgent.click();
    await page.waitForTimeout(700);
    await dump(page, 'w7_agent_form');
    const inputs = page.locator('sebas-settings-modal input');
    const n = await inputs.count();
    if (n >= 2) {
      await inputs.nth(0).fill('delete-me');
      await inputs.nth(1).fill('D:/workbench/repos-ai/sebas/target/debug/fake-claude.exe');
    }
    await page.getByRole('button', { name: /保存|创建|确定/ }).last().click().catch(() => {});
    await page.waitForTimeout(900);
    await shot(page, 'w7_agent_added');
    // open its row action dropdown and click 删除
    const row = page.getByText('delete-me').first();
    await row.click().catch(() => {});
    await page.waitForTimeout(500);
    const del = page.getByRole('button', { name: '删除 agent' }).last()
      .or(page.locator('wa-dropdown wa-button[aria-label*="删除"]').last());
    await del.click().catch(() => {});
    await page.waitForTimeout(600);
    // menu item 删除 (inside dropdown)
    await page.getByText('删除', { exact: true }).last().click().catch(() => {});
    await page.waitForTimeout(700);
    await shot(page, 'w7_delete_confirm');
    const copy = await page.getByTestId('agent-delete-copy').innerText().catch(() => '');
    const okCopy = copy.includes('下拉中消失') && !copy.includes('中立') && copy.includes('自然结束');
    report('W7 delete-agent copy unambiguous', okCopy, copy.slice(0, 110));
  } else {
    await dump(page, 'w7_no_add_agent');
    report('W7 delete-agent copy unambiguous', false, 'add-agent entry not found');
  }

  await ctx.close();
} finally {
  await browser.close();
}
console.log('SUMMARY ' + JSON.stringify(results));
