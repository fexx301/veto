// Veto operator console page logic (served as /app.js; no inline script).
const actions = ["execute", "upgrade", "probe", "fix", "reapprove", "resume"];
const validActions = new Set(["approve", "start", "start-demo", ...actions]);
const errorBanner = document.getElementById("action-error");
const sessionLedger = document.getElementById("session-ledger");
const workflowAnnouncer = document.getElementById("workflow-announcer");
const copyAnnouncer = document.getElementById("copy-announcer");
let busy = false;
let eventCount = 0;
let lastAnnouncement = "";
let lastStatus = null;

function setError(message) {
  errorBanner.textContent = message || "";
  errorBanner.classList.toggle("is-visible", Boolean(message));
}

function nextAction(phase, data = lastStatus) {
  if (data?.hosted && data.canStart && (phase === "idle" || phase === "resumed" || !data.owned)) return "start";
  if (data?.hosted && !data.owned) return "";
  return ({ awaitingApproval: "approve", ready: "execute", executed: "upgrade", upgraded: "probe", blocked: "fix", fixed: "reapprove", reapproved: "resume" })[phase] || "";
}

function explorerUrl(kind, value) {
  if (lastStatus?.network !== "devnet" || !value) return "";
  return "https://explorer.solana.com/" + kind + "/" + value + "?cluster=devnet";
}

function phaseMessage(phase) {
  return ({
    initializing: ["Starting session", "loading"],
    idle: ["Start a run to try it", "ready"],
    awaitingApproval: ["Your approval is required", "wallet"],
    ready: ["Reviewed build approved", "aligned"],
    executed: ["Both wallets paid 10", "aligned"],
    upgraded: ["Code changed under the same address", "changed"],
    blocked: ["Veto blocked the payment", "blocked"],
    fixed: ["Reviewed build redeployed", "review"],
    reapproved: ["New deployment approved", "aligned"],
    resumed: ["Payments resumed", "complete"]
  })[phase] || ["Session", "ready"];
}

function shortAddress(address) {
  if (!address) return "—";
  return address.length > 22 ? address.slice(0, 9) + "…" + address.slice(-7) : address;
}

function showAddress(id, address) {
  const el = document.getElementById(id);
  el.textContent = shortAddress(address);
  el.dataset.copyValue = address || "";
  el.disabled = !address;
  if (address) el.title = "Select to copy " + address;
  else el.removeAttribute("title");
}

function stepState(key, phase) {
  if (phase === "awaitingApproval" || phase === "idle") return "pending";
  const currentIndex = actions.indexOf(nextAction(phase));
  const index = actions.indexOf(key);
  if (phase === "resumed") return key === "probe" ? "blocked" : "complete";
  if (currentIndex === index) return "active";
  if (index < currentIndex) return key === "probe" ? "blocked" : "complete";
  return "pending";
}

function actionLabel(action) {
  return ({
    "start-demo": "Try it — no wallet needed",
    start: "Connect wallet & start",
    approve: "Connect wallet & approve",
    execute: "Pay 10 from both",
    upgrade: "Upgrade merchant-pay",
    probe: "Request 10 from both",
    fix: "Redeploy reviewed build",
    reapprove: "Review & sign deployment",
    resume: "Pay 10 through Veto"
  })[action] || "Workflow complete";
}

function announceWorkflow(data, headline, enabled) {
  const message = headline[0] + ". Approved slot " + (data.approvedSlot ?? "unavailable") + ", current slot " + (data.currentSlot ?? "unavailable") + ". " + (enabled ? actionLabel(enabled) + " available." : "Workflow complete.");
  if (message !== lastAnnouncement) {
    lastAnnouncement = message;
    workflowAnnouncer.textContent = message;
  }
}

function renderStatus(data) {
  lastStatus = data;
  let headline = phaseMessage(data.phase);
  if (data.hosted && !data.owned && data.phase !== "idle" && !data.canStart) headline = ["Another visitor is running the demo", "watching"];
  document.getElementById("deployment-state").textContent = headline[0];
  document.getElementById("state-chip").textContent = headline[1];
  document.getElementById("header-phase").textContent = headline[1];
  showAddress("operator-id", data.operator);
  // A demo-operator run is approved by the server's test key, not the visitor.
  const demoOperator = data.externalOperator === false;
  document.querySelector("#node-human .node-name").textContent = demoOperator ? "Demo operator" : "Your wallet";
  document.querySelector("#node-human .node-role").textContent = demoOperator ? "Server test key · sole approver" : "Wallet root · sole approver";
  document.querySelector('[data-step="reapprove"] .step-copy').textContent = demoOperator
    ? "The code matches the build you reviewed. The demo operator's test key signs; with your own wallet, you would."
    : "The code matches the build you reviewed. Your wallet signs.";
  ["target-id", "target-ledger"].forEach((id) => showAddress(id, data.target));
  ["swig-config", "swig-ledger"].forEach((id) => showAddress(id, data.swigConfig));
  ["policy-id", "policy-ledger"].forEach((id) => showAddress(id, data.policy));
  document.getElementById("approved-slot").textContent = data.approvedSlot ?? "—";
  const current = document.getElementById("current-slot");
  current.textContent = data.currentSlot ?? "—";
  const stale = data.approvedSlot !== null && data.currentSlot !== data.approvedSlot;
  current.classList.toggle("is-stale", stale);
  ["plain-ledger"].forEach((id) => showAddress(id, data.plainSwigConfig));
  ["agent-ledger"].forEach((id) => showAddress(id, data.agent));
  ["mint-ledger"].forEach((id) => showAddress(id, data.mint));
  // Before a run, show the starting budgets rather than empty dashes.
  const idle = data.phase === "idle";
  const balances = idle ? { veto: data.budget, plain: data.budget, merchant: "0.00" } : (data.balances || {});
  document.getElementById("scenario-hint").hidden = !idle;
  document.getElementById("balance-veto").textContent = balances.veto ?? "—";
  document.getElementById("balance-plain").textContent = balances.plain ?? "—";
  document.getElementById("balance-merchant").textContent = balances.merchant ?? "—";
  document.getElementById("budget-copy").textContent = String(data.budget ?? "500").replace(/\.00$/, "");
  const plainDrained = balances.plain === "0.00";
  document.getElementById("cell-plain").dataset.status = plainDrained ? "drained" : "";
  document.getElementById("cell-veto").dataset.status = plainDrained ? "protected" : "";
  const matches = Boolean(data.codeMatchesReviewed);
  document.getElementById("code-state").textContent = matches ? "Reviewed build" : "Not reviewed";
  document.getElementById("code-hash").textContent = "sha256 " + (data.currentCodeHash ? data.currentCodeHash.slice(0, 12) + "…" : "—");
  document.getElementById("code-hash").title = data.currentCodeHash || "";
  document.getElementById("cell-code").dataset.status = matches ? "" : "mismatch";
  const onDevnet = data.network === "devnet";
  document.getElementById("connection-copy").textContent = onDevnet ? "Solana devnet" : "Local validator";
  document.getElementById("footer-network").textContent = onDevnet ? "Solana devnet demo" : "local validator demo";
  document.getElementById("network-label").textContent = onDevnet ? "Live devnet state" : "Live local state";
  document.getElementById("sequence-progress").textContent = data.phase === "resumed" ? "Complete" : "Live run";
  // Colour only what has happened: before a run everything is neutral,
  // "active" marks whose turn it is, green means approved or completed.
  const approved = data.approvedSlot !== null && data.approvedSlot !== undefined;
  const needsHuman = data.phase === "awaitingApproval" || data.phase === "fixed";
  const setStatus = (id, status) => {
    const node = document.getElementById(id);
    if (status) node.dataset.status = status;
    else node.removeAttribute("data-status");
  };
  setStatus("node-human", needsHuman ? "active" : (approved ? "verified" : ""));
  setStatus("node-target", !matches ? "blocked" : (data.phase === "resumed" ? "verified" : ""));
  setStatus("node-policy", !approved ? "" : (stale ? "blocked" : (data.phase === "reapproved" || data.phase === "resumed" ? "verified" : "")));
  setStatus("node-wallet", !approved ? "" : (data.phase === "executed" || data.phase === "resumed" ? "verified" : ""));
  const enabled = nextAction(data.phase, data);
  const startDemo = document.getElementById("start-demo");
  startDemo.hidden = enabled !== "start";
  startDemo.disabled = busy;
  document.getElementById("wallet-hint").hidden = enabled !== "start";
  const currentAction = document.getElementById("current-action");
  // Most visitors have no devnet wallet: the demo operator is the primary path.
  const primary = enabled === "start" ? "start-demo" : enabled;
  currentAction.dataset.currentAction = primary;
  currentAction.textContent = actionLabel(primary);
  // Never offer an approval for code that differs from the reviewed build.
  const approvalBlocked = (enabled === "approve" || enabled === "reapprove") && !matches;
  if (approvalBlocked) currentAction.textContent = "Deployed code not reviewed";
  currentAction.disabled = busy || !enabled || approvalBlocked;
  currentAction.setAttribute("aria-busy", busy ? "true" : "false");
  for (const key of actions) {
    const row = document.querySelector('[data-step="' + key + '"]');
    const state = stepState(key, data.phase);
    row.dataset.state = state;
    row.querySelector(".step-status").textContent = state === "complete" ? "Done" : state === "blocked" ? "Blocked" : state === "active" ? "Next" : "Waiting";
  }
  for (const button of document.querySelectorAll("[data-action]")) {
    button.disabled = busy || button.dataset.action !== enabled || approvalBlocked;
    button.setAttribute("aria-busy", busy && button.dataset.action === enabled ? "true" : "false");
  }
  announceWorkflow(data, headline, enabled);
}

function renderUnavailable() {
  document.getElementById("deployment-state").textContent = "State unavailable";
  document.getElementById("state-chip").textContent = "Offline";
  document.getElementById("header-phase").textContent = "Offline";
  document.getElementById("sequence-progress").textContent = "Unavailable";
  const currentAction = document.getElementById("current-action");
  currentAction.dataset.currentAction = "";
  currentAction.textContent = "State unavailable";
  currentAction.disabled = true;
  document.querySelectorAll("[data-action]").forEach((button) => button.disabled = true);
  document.querySelectorAll(".step").forEach((row) => {
    row.dataset.state = "pending";
    row.querySelector(".step-status").textContent = "Unavailable";
  });
  document.querySelectorAll(".trust-node").forEach((node) => node.removeAttribute("data-status"));
  const message = "Veto state unavailable. Transaction actions are disabled.";
  if (message !== lastAnnouncement) {
    lastAnnouncement = message;
    workflowAnnouncer.textContent = message;
  }
}

function appendEvent(action, data) {
  eventCount += 1;
  document.getElementById("session-empty").hidden = true;
  document.getElementById("record-kind").textContent = eventCount + " event" + (eventCount === 1 ? "" : "s");
  const meta = {
    start: ["Visitor", "Started a run", "ready", "success"],
    "start-demo": ["Visitor", "Started a run (demo operator)", "ready", "success"],
    approve: ["Operator", "Initial approval", "approved", "success"],
    execute: ["Agent", "Paid 10 from both", "allowed", "success"],
    upgrade: ["Protocol", "Shipped v2", "code changed", "warning"],
    probe: ["Agent · Veto", "Payment request", "blocked", "danger"],
    fix: ["Protocol", "Redeployed reviewed build", "slot changed", "warning"],
    reapprove: ["Operator", "Approve deployment", "approved", "success"],
    resume: ["Agent", "Paid 10 through Veto", "allowed", "success"]
  }[action];
  const signature = (data.latest.detail.match(/Signature:\s*([1-9A-HJ-NP-Za-km-z]+)/) || [])[1] || "local transaction";
  const row = document.createElement("li");
  row.className = "event-row";
  row.dataset.actor = meta[0];
  row.dataset.tone = meta[3];
  const link = explorerUrl("tx", signature === "local transaction" ? "" : signature);
  const plainSignature = (data.latest.detail.match(/Plain allowlist signature:\s*([1-9A-HJ-NP-Za-km-z]+)/) || [])[1];
  const plainLink = explorerUrl("tx", plainSignature);
  // Built with DOM nodes and textContent only: no server-supplied value is
  // ever parsed as HTML.
  const el = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = String(text);
    return node;
  };
  const operation = el("div", "event-operation");
  operation.append(el("strong", "", meta[0] + " · " + meta[1]));
  const copy = el("button", "copy-value", shortAddress(signature));
  copy.type = "button";
  copy.dataset.copyValue = signature;
  copy.setAttribute("aria-label", "Copy full transaction signature");
  operation.append(copy);
  for (const [href, text] of [[link, "explorer ↗"], [plainLink, "plain wallet tx ↗"]]) {
    if (!href) continue;
    const anchor = el("a", "explorer-link", text);
    anchor.href = href;
    anchor.target = "_blank";
    anchor.rel = "noopener";
    operation.append(anchor);
  }
  const slots = el("div", "event-slots");
  slots.append(el("span", "", "approved " + data.approvedSlot), el("span", "", "observed " + data.currentSlot));
  row.append(el("span", "event-index", String(eventCount).padStart(2, "0")), operation, slots, el("span", "event-verdict", meta[2]));
  sessionLedger.appendChild(row);
}

async function refresh(clearExistingError = true) {
  try {
    const response = await fetch("/api/status", { cache: "no-store" });
    if (!response.ok) throw new Error("Local server returned " + response.status);
    renderStatus(await response.json());
    const connection = document.getElementById("connection-state");
    connection.dataset.status = "connected";
    if (clearExistingError) setError("");
  } catch (error) {
    const connection = document.getElementById("connection-state");
    connection.dataset.status = "disconnected";
    document.getElementById("connection-copy").textContent = "Disconnected";
    renderUnavailable();
    setError("Cannot read the Veto server: " + error.message);
  }
}

async function signOperatorApproval(action) {
  if (!lastStatus?.externalOperator) throw new Error("This session is not configured for an external operator wallet.");
  const preparedResponse = await fetch("/api/operator/approval-transaction", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
  const prepared = await preparedResponse.json();
  if (!preparedResponse.ok) throw new Error(prepared.error || "Could not prepare the approval transaction.");
  const signed = await VetoWallet.signApproval(prepared.transaction, prepared.operator, prepared.gate);
  const submitResponse = await fetch("/api/operator/submit-approval", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ transaction: signed }) });
  const data = await submitResponse.json();
  if (!submitResponse.ok) throw new Error(data.error || "Signed approval was rejected.");
  renderStatus(data);
  appendEvent(action, data);
}

async function runAction(action) {
  if (busy || !validActions.has(action)) return;
  busy = true;
  setError("");
  let actionFailed = false;
  const actionButtons = [...document.querySelectorAll('[data-action="' + action + '"]'), document.getElementById("current-action")];
  const priorTexts = actionButtons.map((button) => button.textContent.trim());
  document.querySelectorAll("[data-action],#current-action,#start-demo").forEach((button) => button.disabled = true);
  actionButtons.forEach((button) => {
    button.dataset.busy = "true";
    button.setAttribute("aria-busy", "true");
    button.textContent = (action === "start" || action === "approve" || (action === "reapprove" && lastStatus?.externalOperator)) ? "Waiting for wallet…" : (action === "start-demo" ? "Creating wallets…" : "Submitting…");
  });
  try {
    if (action === "start" || action === "start-demo") {
      const operator = action === "start" ? await VetoWallet.connectWallet() : "demo";
      const response = await fetch("/api/run", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ operator }) });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error || "Could not start a run");
      sessionLedger.replaceChildren();
      eventCount = 0;
      renderStatus(data);
      appendEvent(action, data);
    } else if (action === "approve" || (action === "reapprove" && lastStatus?.externalOperator)) {
      await signOperatorApproval(action);
    } else {
      const response = await fetch("/api/action/" + action, { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error || "Action failed");
      renderStatus(data);
      appendEvent(action, data);
    }
  } catch (error) {
    actionFailed = true;
    setError(error.message);
  } finally {
    busy = false;
    actionButtons.forEach((button, index) => {
      button.dataset.busy = "false";
      button.setAttribute("aria-busy", "false");
      button.textContent = priorTexts[index];
    });
    // Restoring the pre-click text would leave the main button showing the
    // previous step until the next status refresh; redraw from the latest
    // status immediately so its label always matches what it will do.
    if (lastStatus) renderStatus(lastStatus);
    await refresh(!actionFailed);
  }
}

document.querySelectorAll("[data-action]").forEach((button) => button.addEventListener("click", () => runAction(button.dataset.action)));
document.getElementById("current-action").addEventListener("click", (event) => runAction(event.currentTarget.dataset.currentAction));
document.getElementById("start-demo").addEventListener("click", () => runAction("start"));
document.addEventListener("click", async (event) => {
  const button = event.target.closest("[data-copy-value]");
  if (!button || !button.dataset.copyValue) return;
  try {
    await navigator.clipboard.writeText(button.dataset.copyValue);
    button.textContent = "Copied";
    copyAnnouncer.textContent = "Copied full value to clipboard.";
    setTimeout(() => button.textContent = shortAddress(button.dataset.copyValue), 900);
  } catch (error) {
    copyAnnouncer.textContent = "Copy failed.";
    setError("Could not copy this value. " + error.message);
  }
});
refresh();
