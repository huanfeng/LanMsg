// 网页临时会话真实浏览器 e2e（Chromium headless）。用法: node run.mjs <mode: key|code> <sock> <outdir>
// 宿主侧动作通过 ipc.py 发给 lanmsgd；webhost.py 已在后台自动允许访客。
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
// PLAYWRIGHT_CORE：playwright-core 的路径（默认按包名解析）；CHROMIUM：浏览器可执行文件（默认用 Playwright 自带的）。
const { chromium } = require(process.env.PLAYWRIGHT_CORE || "playwright-core");

const [mode, SOCK, OUT] = process.argv.slice(2);
const IPC = path.join(path.dirname(fileURLToPath(import.meta.url)), "ipc.py");
const session = JSON.parse(fs.readFileSync(path.join(OUT, "session.json"), "utf8")).Ok.WebSession.value;
const results = [];
const sha = (p) => createHash("sha256").update(fs.readFileSync(p)).digest("hex");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
function step(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " — " + detail : ""}`);
}
function host(body) {
  const out = execFileSync("python3", ["-I", IPC, SOCK, JSON.stringify(body)], { encoding: "utf8" });
  return JSON.parse(out);
}
async function until(fn, ms = 30000, every = 300) {
  const t0 = Date.now();
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() - t0 > ms) return null;
    await sleep(every);
  }
}
// 本次运行开始前已存在的访客会话（上一轮模式留下的），本轮要找的是新出现的那个。
const knownGuests = new Set(
  (host("ListConversations").Ok?.Conversations?.value ?? []).filter((c) => c.kind === "Guest").map((c) => c.conv));
function guestConv() {
  const r = host("ListConversations");
  const list = r.Ok?.Conversations?.value ?? [];
  return list.find((c) => c.kind === "Guest" && !knownGuests.has(c.conv))?.conv ?? null;
}
function guestMessages(conv) {
  const r = host({ ListMessages: { conv, before: null, limit: 100 } });
  return r.Ok?.Messages?.value ?? [];
}

const browser = await chromium.launch({
  ...(process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {}),
  headless: true,
  args: ["--no-sandbox"],
});
const ctx = await browser.newContext({ acceptDownloads: true, viewport: { width: 420, height: 800 } });
const page = await ctx.newPage();
const errors = [];
page.on("console", (m) => { if (m.type() === "error") errors.push("console: " + m.text()); });
page.on("pageerror", (e) => errors.push("pageerror: " + e.message));
page.on("requestfailed", (r) => errors.push("requestfailed: " + r.url() + " " + (r.failure()?.errorText ?? "")));

try {
  // ---- 1. 进入会话 ----
  const base = "http://127.0.0.1:53321/";
  if (mode === "key") {
    const hash = new URL(session.url).hash;
    await page.goto(base + hash);
  } else {
    await page.goto(base);
    const form = await until(async () => (await page.locator("#code-form").isVisible()) || null, 15000);
    step("code 模式：出现配对码输入框", !!form);
    await page.screenshot({ path: path.join(OUT, `${mode}-0-gate.png`) });
    await page.fill("#code-input", session.code);
    await page.click("#code-submit");
  }
  const chatOk = await until(async () => (await page.locator("#chat").isVisible()) || null, 40000);
  await page.screenshot({ path: path.join(OUT, `${mode}-1-chat.png`) });
  step(`${mode} 模式：握手并经宿主允许后进入聊天页`, !!chatOk,
    chatOk ? "" : "gate=" + (await page.locator("#gate-text").innerText().catch(() => "?")));
  if (!chatOk) throw new Error("没有进入聊天页");
  const dotOn = await until(async () => ((await page.locator("#dot").getAttribute("class")) || "").includes("on") || null, 15000);
  step("连接状态指示为在线", !!dotOn);

  // ---- 2. 浏览器 → 宿主：文字 ----
  const text = "你好 from browser ✓ " + mode;
  await page.fill("#text-input", text);
  await page.click("#send-btn");
  const conv = await until(() => guestConv(), 15000);
  step("宿主出现访客会话", !!conv);
  const gotText = await until(() => (conv && guestMessages(conv).some((m) => m.body?.Text?.text === text)) || null, 15000);
  step("宿主收到浏览器发的文字（含中文与符号）", !!gotText);

  // ---- 3. 浏览器 → 宿主：文件（中文名，3.5 MB，多分片加密上传） ----
  const upDir = path.join(OUT, "up"); fs.mkdirSync(upDir, { recursive: true });
  const upFile = path.join(upDir, `测试文件-${mode}-${Date.now()}.bin`);
  fs.writeFileSync(upFile, randomBytes(3.5 * 1024 * 1024));
  await page.setInputFiles("#file-input", upFile);
  const jobGone = await until(async () => ((await page.locator("#jobs li").count()) === 0) || null, 90000, 500);
  step("浏览器上传任务完成（任务列表清空）", !!jobGone,
    jobGone ? "" : "jobs=" + (await page.locator("#jobs").innerText().catch(() => "?")));
  const upMsg = await until(() => {
    if (!conv) return null;
    return guestMessages(conv).find((m) => (m.body?.Files?.items ?? []).some((i) => i.name === path.basename(upFile))) || null;
  }, 20000);
  step("宿主会话里出现该文件消息（保留中文名）", !!upMsg);
  const item = upMsg?.body?.Files?.items?.find((i) => i.name === path.basename(upFile));
  const localPath = item?.local_path;
  step("宿主落盘文件的 SHA-256 与原文件一致", !!localPath && fs.existsSync(localPath) && sha(localPath) === sha(upFile),
    localPath ? `${localPath} size=${item.size}` : "无 local_path");
  await page.screenshot({ path: path.join(OUT, `${mode}-2-after-upload.png`) });

  // ---- 4. 宿主 → 浏览器：文字 + 文件 ----
  const hostText = "host says hi 宿主回复 " + mode;
  host({ SendText: { conv, text: hostText } });
  const textSeen = await until(async () => (await page.locator("#list .bubble", { hasText: hostText }).count()) > 0 || null, 20000);
  step("浏览器收到宿主文字", !!textSeen);

  const dnDir = path.join(OUT, "down"); fs.mkdirSync(dnDir, { recursive: true });
  const dnFile = path.join(dnDir, `host-文件-${mode}-${Date.now()}.bin`);
  fs.writeFileSync(dnFile, randomBytes(5 * 1024 * 1024));
  host({ SendFiles: { conv, paths: [dnFile] } });
  const fileBubble = page.locator("#list .bubble", { hasText: path.basename(dnFile) });
  const fileSeen = await until(async () => (await fileBubble.count()) > 0 || null, 30000);
  step("浏览器显示宿主发来的文件卡片", !!fileSeen);
  if (fileSeen) {
    const [dl] = await Promise.all([
      page.waitForEvent("download", { timeout: 90000 }),
      fileBubble.locator("button").click(),
    ]);
    const saved = path.join(dnDir, "got-" + mode + ".bin");
    await dl.saveAs(saved);
    step("浏览器下载文件内容与宿主原文件一致（解密+分块拼接）", sha(saved) === sha(dnFile),
      `suggested=${dl.suggestedFilename()}`);
  }
  await page.screenshot({ path: path.join(OUT, `${mode}-3-final.png`) });

  // ---- 5. 宿主停止会话 → 页面应显示“已结束”并回到门页 ----
  if (process.env.STOP) {
    host("WebStop");
    const ended = await until(async () => (await page.locator("#gate").isVisible()) || null, 20000);
    const gateText = ended ? await page.locator("#gate-text").innerText() : "";
    const formShown = ended ? await page.locator("#code-form").isVisible() : false;
    step("宿主停止会话后页面显示已结束（不再提供输入框）", !!ended && gateText.length > 0 && !formShown, `gate="${gateText}"`);
    await page.screenshot({ path: path.join(OUT, `${mode}-4-ended.png`) });
  }
} catch (e) {
  step("脚本异常", false, String(e));
  await page.screenshot({ path: path.join(OUT, `${mode}-err.png`) }).catch(() => {});
}
step("浏览器无 console error / pageerror / 请求失败", errors.length === 0, errors.slice(0, 5).join(" | "));
await browser.close();
const bad = results.filter((r) => !r.ok).length;
fs.writeFileSync(path.join(OUT, `${mode}-results.json`), JSON.stringify(results, null, 2));
console.log(`\n${mode} 模式：${results.length - bad}/${results.length} 通过`);
process.exit(bad ? 1 : 0);
