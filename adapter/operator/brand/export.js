// Renders brand PNGs from the SVGs with Chrome so the lockup uses Geist.
// Run from adapter/operator: node brand/export.js
const fs = require("node:fs");
const path = require("node:path");
const { chromium } = require("playwright");

const dir = path.join(__dirname);
const svg = (name) => fs.readFileSync(path.join(dir, name), "utf8");
const fonts = '<link href="https://fonts.googleapis.com/css2?family=Geist:wght@600;700&display=block" rel="stylesheet">';
const page = (body, bg) => `<!doctype html><html><head>${fonts}<style>html,body{margin:0;background:${bg}}svg{display:block}</style></head><body>${body}</body></html>`;

(async () => {
  const browser = await chromium.launch({ channel: "chrome", headless: true });
  const tab = await browser.newPage({ deviceScaleFactor: 1 });
  async function shot(html, width, height, file, transparent = false) {
    await tab.setViewportSize({ width: Math.max(width, 1200), height: Math.max(height, 1000) });
    await tab.setContent(html, { waitUntil: "networkidle" });
    await tab.evaluate(() => document.fonts.ready);
    await tab.screenshot({ path: path.join(dir, "png", file), omitBackground: transparent, clip: { x: 0, y: 0, width, height } });
  }
  for (const size of [16, 32, 64, 180, 256, 512, 1024]) {
    await shot(page(svg("veto-mark.svg").replace("<svg ", `<svg width="${size}" height="${size}" `), "transparent"), size, size, `veto-mark-${size}.png`, true);
  }
  for (const [name, bg] of [["veto-mark-ink.svg", "transparent"], ["veto-mark-reverse.svg", "#1D130F"]]) {
    const pad = name.includes("reverse") ? 64 : 0;
    await shot(page(`<div style="padding:${pad}px">${svg(name).replace("<svg ", '<svg width="512" height="512" ')}</div>`, bg), 512 + 2 * pad, 512 + 2 * pad, name.replace(".svg", "-512.png"), bg === "transparent");
  }
  // Social avatar: mark centred on paper, square.
  await shot(page(`<div style="width:400px;height:400px;display:grid;place-items:center">${svg("veto-mark.svg").replace("<svg ", '<svg width="280" height="280" ')}</div>`, "#FDF8F3"), 400, 400, "veto-avatar-400.png");
  for (const [name, bg] of [["veto-lockup.svg", "#FDF8F3"], ["veto-lockup-on-dark.svg", "#1D130F"]]) {
    await shot(page(`<div style="padding:60px 80px">${svg(name).replace("<svg ", '<svg width="1040" height="287" ')}</div>`, bg), 1200, 407, name.replace(".svg", "-1200.png"));
  }
  // Review sheet: sizes and variants side by side.
  const sizes = [512, 128, 64, 32, 16].map((s) => `<div style="display:grid;place-items:center;gap:8px">${svg("veto-mark.svg").replace("<svg ", `<svg width="${s}" height="${s}" `)}<span style="font:12px Geist,sans-serif;color:#6b5f58">${s}px</span></div>`).join("");
  const variants = ["veto-mark.svg", "veto-mark-ink.svg"].map((n) => svg(n).replace("<svg ", '<svg width="120" height="120" ')).join("") +
    `<div style="background:#1D130F;padding:20px">${svg("veto-mark-reverse.svg").replace("<svg ", '<svg width="120" height="120" ')}</div>`;
  const sheet = `<div style="padding:48px;display:grid;gap:48px"><div style="display:flex;align-items:end;gap:40px">${sizes}</div><div style="display:flex;align-items:center;gap:40px">${variants}</div><div style="display:flex;gap:40px;align-items:center">${svg("veto-lockup.svg").replace("<svg ", '<svg width="464" height="128" ')}<div style="background:#1D130F;padding:24px">${svg("veto-lockup-on-dark.svg").replace("<svg ", '<svg width="464" height="128" ')}</div></div></div>`;
  await shot(page(sheet, "#FDF8F3"), 1200, 1000, "review-sheet.png");
  await browser.close();
  console.log("exported", fs.readdirSync(path.join(dir, "png")).join(" "));
})().catch((error) => { console.error(error); process.exit(1); });
