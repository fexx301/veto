const assert = require("node:assert/strict");
const fs = require("node:fs");
const { chromium } = require("playwright");
const { Keypair, Transaction } = require("@solana/web3.js");

const operatorKeypairPath = process.env.OPERATOR_KEYPAIR;
const externalOperator = operatorKeypairPath
  ? Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(operatorKeypairPath, "utf8"))))
  : null;

function parseOklch(value) {
  const match = value.match(/oklch\(([\d.]+)%\s+([\d.]+)\s+([\d.]+)/);
  assert.ok(match, `could not parse ${value}`);
  return [Number(match[1]) / 100, Number(match[2]), Number(match[3])];
}

function luminance([lightness, chroma, hue]) {
  const radians = hue * Math.PI / 180;
  const a = chroma * Math.cos(radians);
  const b = chroma * Math.sin(radians);
  const lRoot = lightness + 0.3963377774 * a + 0.2158037573 * b;
  const mRoot = lightness - 0.1055613458 * a - 0.0638541728 * b;
  const sRoot = lightness - 0.0894841775 * a - 1.291485548 * b;
  const l = lRoot ** 3, m = mRoot ** 3, s = sRoot ** 3;
  const channels = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ].map((channel) => Math.max(0, Math.min(1, channel)));
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(first, second) {
  const a = luminance(parseOklch(first));
  const b = luminance(parseOklch(second));
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

async function pressCurrentAction(page) {
  const action = page.locator("#current-action");
  await action.focus();
  assert.equal(await action.evaluate((element) => document.activeElement === element), true);
  await page.keyboard.press("Enter");
}

(async () => {
  const browser = await chromium.launch({ channel: "chrome", headless: true });
  const disconnected = await browser.newPage({ viewport: { width: 320, height: 700 } });
  let offlinePosts = 0;
  let heldStatusRoute;
  let releaseHeldStatus;
  const statusWasHeld = new Promise((resolve) => { releaseHeldStatus = resolve; });
  disconnected.on("request", (request) => { if (request.method() === "POST") offlinePosts += 1; });
  await disconnected.route("**/api/status", (route) => { heldStatusRoute = route; releaseHeldStatus(); });
  await disconnected.goto("http://127.0.0.1:4173", { waitUntil: "domcontentloaded" });
  await statusWasHeld;
  assert.equal(await disconnected.locator('[data-copy-value=""]:enabled').count(), 0);
  assert.equal(await disconnected.locator("[data-action]:enabled,#current-action:enabled").count(), 0);
  await heldStatusRoute.abort();
  await disconnected.locator("#connection-copy").waitFor({ state: "visible" });
  await disconnected.waitForFunction(() => document.querySelector("#connection-copy")?.textContent === "Disconnected");
  assert.equal(await disconnected.locator("#connection-copy").textContent(), "Disconnected");
  assert.ok(await disconnected.locator("#action-error").isVisible());
  assert.equal(await disconnected.locator("[data-action]:enabled,#current-action:enabled").count(), 0);
  assert.equal(await disconnected.locator(".step-status", { hasText: "Next" }).count(), 0);
  assert.equal(await disconnected.locator(".trust-node[data-status]").count(), 0);
  assert.equal(await disconnected.locator('[data-copy-value=""]:enabled').count(), 0);
  await disconnected.keyboard.press("Enter");
  assert.equal(offlinePosts, 0);
  const errorTop = await disconnected.locator("#action-error").evaluate((element) => element.getBoundingClientRect().top);
  assert.ok(errorTop < 100, `mobile connection error starts at ${errorTop}px`);
  await disconnected.screenshot({ path: "/tmp/veto-disconnected-mobile.png", fullPage: true });
  await disconnected.close();

  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  if (externalOperator) {
    await page.exposeFunction("signOperatorTransaction", (encoded) => {
      const transaction = Transaction.from(Buffer.from(encoded, "base64"));
      transaction.partialSign(externalOperator);
      return Buffer.from(transaction.serialize()).toString("base64");
    });
    await page.addInitScript((operator) => {
      const publicKey = { toString: () => operator };
      window.solana = {
        publicKey,
        connect: async () => ({ publicKey }),
        signTransaction: async (transaction) => {
          const encoded = window.VetoWallet.serializeTransaction(transaction, false);
          const signed = await window.signOperatorTransaction(encoded);
          return window.VetoWallet.deserializeTransaction(signed);
        },
      };
    }, externalOperator.publicKey.toString());
  }
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));

  await page.goto("http://127.0.0.1:4173", { waitUntil: "networkidle" });
  await page.locator("#connection-copy").waitFor({ state: "visible" });
  assert.equal(await page.locator("#connection-copy").textContent(), "Local validator");
  if (externalOperator) {
    assert.equal(await page.locator("#header-phase").textContent(), "wallet");
    assert.equal(await page.locator("#current-action").textContent(), "Connect wallet & approve");
    await pressCurrentAction(page);
    await page.waitForFunction(() => document.querySelector("#header-phase")?.textContent === "aligned");
    assert.match(await page.locator("#session-ledger").textContent(), /Operator · Initial approval/);
  }
  assert.equal(await page.locator("#approved-slot").textContent(), "0");
  assert.equal(await page.locator("#current-slot").textContent(), "0");
  const firstFold = await page.evaluate(() => ({
    readoutBottom: document.querySelector(".trust-readout").getBoundingClientRect().bottom,
    actionBottom: document.querySelector("#current-action").getBoundingClientRect().bottom,
  }));
  assert.ok(firstFold.readoutBottom <= 800, `trust readout ends at ${firstFold.readoutBottom}px`);
  assert.ok(firstFold.actionBottom <= 800, `first action ends at ${firstFold.actionBottom}px`);
  const colors = await page.evaluate(() => {
    const styles = getComputedStyle(document.documentElement);
    return Object.fromEntries(["paper", "paper-3", "ink", "muted", "accent", "accent-ink", "success", "success-soft", "warning", "warning-soft", "danger", "danger-soft", "focus"].map((name) => [name, styles.getPropertyValue(`--color-${name}`).trim()]));
  });
  const contrastPairs = [["accent", "paper"], ["accent-ink", "accent"], ["success", "success-soft"], ["warning", "warning-soft"], ["danger", "danger-soft"], ["muted", "paper-3"], ["paper", "ink"], ["focus", "paper"]];
  for (const [foreground, background] of contrastPairs) {
    const ratio = contrast(colors[foreground], colors[background]);
    assert.ok(ratio >= 4.5, `${foreground}/${background} contrast is ${ratio.toFixed(2)}:1`);
  }
  await page.screenshot({ path: "/tmp/veto-ready.png", fullPage: true });

  await page.route("**/api/action/execute", (route) => route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "Simulated action rejection" }) }), { times: 1 });
  await pressCurrentAction(page);
  await page.waitForFunction(() => document.querySelector("#action-error")?.textContent === "Simulated action rejection");
  await page.waitForFunction(() => !document.querySelector("#current-action")?.disabled);
  assert.ok(await page.locator("#action-error").isVisible());
  assert.equal(await page.locator("#session-ledger .event-row").count(), externalOperator ? 1 : 0);
  assert.equal(await page.locator("#current-action").isEnabled(), true);

  await pressCurrentAction(page);
  await page.waitForFunction(() => document.querySelector("#balance-veto")?.textContent === "490.00");
  assert.equal(await page.locator("#balance-plain").textContent(), "490.00");
  assert.equal(await page.locator("#code-state").textContent(), "Reviewed build");
  const firstSignature = page.locator("#session-ledger .copy-value").last();
  await firstSignature.focus();
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.querySelector("#copy-announcer")?.textContent === "Copied full value to clipboard.");
  assert.equal(await page.locator("#copy-announcer").textContent(), "Copied full value to clipboard.");

  await pressCurrentAction(page);
  await page.waitForFunction(() => document.querySelector("#header-phase")?.textContent === "changed");
  assert.notEqual(await page.locator("#current-slot").textContent(), "0");
  assert.equal(await page.locator("#code-state").textContent(), "Not reviewed");
  assert.equal(await page.locator("#node-target").getAttribute("data-status"), "blocked");
  await page.screenshot({ path: "/tmp/veto-mismatch.png", fullPage: true });

  await pressCurrentAction(page);
  await page.waitForFunction(() => document.querySelector("#header-phase")?.textContent === "blocked");
  assert.equal(await page.locator("#balance-plain").textContent(), "0.00");
  assert.equal(await page.locator("#balance-veto").textContent(), "490.00");
  assert.equal(await page.locator("#cell-plain").getAttribute("data-status"), "drained");
  assert.equal(await page.locator("#cell-veto").getAttribute("data-status"), "protected");
  await page.screenshot({ path: "/tmp/veto-rejected.png", fullPage: true });

  await pressCurrentAction(page);
  await page.waitForFunction(() => document.querySelector("#header-phase")?.textContent === "review");
  assert.equal(await page.locator("#code-state").textContent(), "Reviewed build");
  assert.equal(await page.locator("#node-human").getAttribute("data-status"), "active");

  await pressCurrentAction(page);
  await page.waitForFunction(() => document.querySelector("#header-phase")?.textContent === "aligned");
  assert.equal(await page.locator("#node-human").getAttribute("data-status"), "verified");
  await page.screenshot({ path: "/tmp/veto-reapproved.png", fullPage: true });

  await pressCurrentAction(page);
  await page.waitForFunction(() => document.querySelector("#header-phase")?.textContent === "complete");
  assert.equal(await page.locator("#balance-veto").textContent(), "480.00");
  assert.equal(await page.locator("#session-ledger .event-row").count(), externalOperator ? 7 : 6);
  assert.match(await page.locator("#session-ledger").textContent(), /Operator · Approve deployment/);
  assert.match(await page.locator("#session-ledger").textContent(), /blocked/);
  await page.screenshot({ path: "/tmp/veto-resumed.png", fullPage: true });
  assert.deepEqual(pageErrors, []);

  for (const width of [320, 375, 414, 768, 1280, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    const dimensions = await page.evaluate(() => ({
      document: document.documentElement.scrollWidth,
      body: document.body.scrollWidth,
      tallestButton: Math.max(...Array.from(document.querySelectorAll("button")).map((button) => button.getBoundingClientRect().height)),
    }));
    assert.equal(dimensions.document, width);
    assert.equal(dimensions.body, width);
    assert.ok(await page.locator("#connection-state").isVisible(), `connection status hidden at ${width}px`);
    assert.ok(dimensions.tallestButton <= 56, `button height ${dimensions.tallestButton}px at ${width}px`);
    console.log(`viewport=${width} document=${dimensions.document} body=${dimensions.body} maxButton=${dimensions.tallestButton}`);
  }
  console.log(`firstFoldReadout=${Math.round(firstFold.readoutBottom)} firstFoldAction=${Math.round(firstFold.actionBottom)} contrastPairs=${contrastPairs.length}`);
  console.log(`actions=${externalOperator ? 7 : 6} ledger=${externalOperator ? 7 : 6} veto=480.00 plain=0.00 keyboard=all copy=announced disconnected=safe pageErrors=0 externalOperator=${Boolean(externalOperator)}`);
  await browser.close();
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
