//! Operator console for the test-token payment demo.
//!
//! The server prepares and fee-pays transactions; the run's operator wallet
//! signs every Veto approval and is the Swig root of the protected wallet.
//! Locally (default) it serves one run on loopback. With `HOSTED=1` it serves
//! a public page where each visitor starts their own run, choosing their own
//! wallet or the server's demo operator key; only that visitor can drive it.

#[path = "../demo/payment.rs"]
mod payment;

use {
    anyhow::{anyhow, bail, Context, Result},
    base64::{engine::general_purpose::STANDARD as BASE64, Engine as _},
    payment::*,
    serde_json::{json, Value},
    solana_client::rpc_client::RpcClient,
    solana_commitment_config::CommitmentConfig,
    solana_sdk::{
        message::Message,
        pubkey::Pubkey,
        signature::{read_keypair_file, Keypair, Signature},
        signer::Signer,
        transaction::Transaction,
    },
    solana_system_interface::instruction as system_instruction,
    std::{
        collections::VecDeque,
        env,
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, Mutex, TryLockError},
        thread,
        time::{Duration, Instant},
    },
};

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 256 * 1024;
const INDEX_HTML: &str = include_str!("../../operator/index.html");
const TOKENS_CSS: &str = include_str!("../../operator/tokens.css");
const WALLET_CLIENT_JS: &str = include_str!("../../operator/wallet-client.bundle.js");
const FAVICON_SVG: &str = include_str!("../../operator/brand/favicon.svg");
/// A hosted run nobody has touched for this long can be taken over.
const RUN_IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Refuse new hosted runs when the fee payer falls below this balance.
const MIN_SETUP_LAMPORTS: u64 = 300_000_000;
const COOKIE: &str = "veto_run";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    AwaitingApproval,
    Ready,
    Executed,
    Upgraded,
    Blocked,
    Fixed,
    Reapproved,
    Resumed,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingApproval => "awaitingApproval",
            Self::Ready => "ready",
            Self::Executed => "executed",
            Self::Upgraded => "upgraded",
            Self::Blocked => "blocked",
            Self::Fixed => "fixed",
            Self::Reapproved => "reapproved",
            Self::Resumed => "resumed",
        }
    }
}

#[derive(Clone)]
struct PendingApproval {
    transaction: Transaction,
    reviewed_slot: u64,
    initializes_policy: bool,
}

/// Configuration and keys shared by every run.
struct Shared {
    rpc: RpcClient,
    rpc_url: String,
    solana_bin: String,
    hosted: bool,
    public_origin: Option<String>,
    bind_addr: String,
    max_runs_per_hour: usize,
    setup: Keypair,
    agent: Keypair,
    protocol_path: String,
    gate: Pubkey,
    target: Pubkey,
    pay_keypair_path: String,
    pay_v1: String,
    pay_v2: String,
    reviewed_hash: String,
    mint: Pubkey,
    merchant: Pubkey,
}

/// One visitor's scenario: two fresh Swig wallets and a fresh Veto policy.
struct Run {
    token: String,
    last_activity: Instant,
    operator: Pubkey,
    policy: Keypair,
    guarded: Guarded,
    guarded_source: Pubkey,
    plain_swig: Pubkey,
    plain_wallet: Pubkey,
    plain_source: Pubkey,
    phase: Phase,
    latest: Value,
    pending_approval: Option<PendingApproval>,
}

struct DemoState {
    shared: Shared,
    run: Option<Run>,
    run_starts: VecDeque<Instant>,
}

fn required_pubkey(name: &str) -> Result<Pubkey> {
    env::var(name)
        .with_context(|| format!("missing {name}"))?
        .parse()
        .with_context(|| format!("invalid {name}"))
}

fn required_keypair(name: &str) -> Result<Keypair> {
    let path = env::var(name).with_context(|| format!("missing {name}"))?;
    read_keypair_file(path).map_err(|error| anyhow!(error.to_string()))
}

fn required_var(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing {name}"))
}

/// An unguessable run token: the address of a fresh random keypair.
fn new_token() -> String {
    Keypair::new().pubkey().to_string()
}

impl Shared {
    fn initialize() -> Result<Self> {
        let rpc_url = env::var("RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".into());
        let rpc = RpcClient::new_with_commitment(&rpc_url, CommitmentConfig::confirmed());
        let solana_bin = env::var("SOLANA_BIN").unwrap_or_else(|_| "solana".into());
        let hosted = env::var("HOSTED").is_ok_and(|value| value == "1");
        let public_origin = env::var("PUBLIC_ORIGIN").ok();
        if hosted && public_origin.is_none() {
            bail!("HOSTED=1 requires PUBLIC_ORIGIN, for example https://veto.example")
        }
        let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:4173".into());
        let max_runs_per_hour = env::var("MAX_RUNS_PER_HOUR")
            .ok()
            .map(|value| value.parse().context("invalid MAX_RUNS_PER_HOUR"))
            .transpose()?
            .unwrap_or(20);
        let setup = required_keypair("HUMAN_PATH")?;
        let agent = required_keypair("AGENT_PATH")?;
        let pay_v1 = required_var("PAY_V1_SO")?;
        let reviewed_hash = file_code_hash(&pay_v1)?;

        // One test-token mint and merchant account serve every run.
        let mint = Keypair::new();
        let merchant_owner = Keypair::new();
        let merchant = Keypair::new();
        let mut ixs = create_mint_ixs(&rpc, &setup.pubkey(), &mint.pubkey(), &setup.pubkey())?;
        ixs.extend(create_token_account_ixs(
            &rpc,
            &setup.pubkey(),
            &merchant.pubkey(),
            &mint.pubkey(),
            &merchant_owner.pubkey(),
        )?);
        send(&rpc, &setup, &[&mint, &merchant], ixs)?;

        Ok(Self {
            rpc,
            rpc_url,
            solana_bin,
            hosted,
            public_origin,
            bind_addr,
            max_runs_per_hour,
            setup,
            agent,
            protocol_path: required_var("PROTOCOL_PATH")?,
            gate: required_pubkey("GATE_ID")?,
            target: required_pubkey("PAY_ID")?,
            pay_keypair_path: required_var("PAY_KEYPAIR")?,
            pay_v1,
            pay_v2: required_var("PAY_V2_SO")?,
            reviewed_hash,
            mint: mint.pubkey(),
            merchant: merchant.pubkey(),
        })
    }

    fn upgrade_to(&self, so: &str) -> Result<(u64, u64, String)> {
        let (_, old_slot) = programdata_and_slot(&self.rpc, &self.target)?;
        let output =
            deploy(&self.solana_bin, &self.rpc_url, &self.protocol_path, &self.pay_keypair_path, so)?;
        let signature = output
            .lines()
            .find_map(|line| line.strip_prefix("Signature: "))
            .unwrap_or("unavailable")
            .to_owned();
        let (_, new_slot) = programdata_and_slot(&self.rpc, &self.target)?;
        if new_slot == old_slot {
            bail!("loader upgrade did not change the merchant-pay deployment slot")
        }
        wait_past_slot(&self.rpc, new_slot)?;
        Ok((old_slot, new_slot, signature))
    }

    /// Starts a run with fresh wallets. A run abandoned mid-scenario may have
    /// left merchant-pay at v2, so the reviewed build is restored first.
    fn start_run(&self, operator: Pubkey) -> Result<Run> {
        if deployed_code_hash(&self.rpc, &self.target)? != self.reviewed_hash {
            self.upgrade_to(&self.pay_v1)?;
        }
        let (_, initial_slot) = programdata_and_slot(&self.rpc, &self.target)?;
        let protocol =
            read_keypair_file(&self.protocol_path).map_err(|error| anyhow!(error.to_string()))?;
        // The protocol team key pays for its own upgrade buffers.
        if self.rpc.get_balance(&protocol.pubkey())? < 500_000_000 {
            send(
                &self.rpc,
                &self.setup,
                &[],
                vec![system_instruction::transfer(
                    &self.setup.pubkey(),
                    &protocol.pubkey(),
                    1_000_000_000,
                )],
            )?;
        }

        let seed: [u8; 16] = Keypair::new().pubkey().to_bytes()[..16].try_into()?;
        let fund = |root: Pubkey, tag: u8| -> Result<(Pubkey, Pubkey, Pubkey)> {
            let mut id = [tag; 32];
            id[..16].copy_from_slice(&seed);
            let (swig, wallet, create) = swig_create(id, self.setup.pubkey(), root)?;
            let source = Keypair::new();
            let mut ixs = vec![create];
            ixs.extend(create_token_account_ixs(
                &self.rpc,
                &self.setup.pubkey(),
                &source.pubkey(),
                &self.mint,
                &wallet,
            )?);
            ixs.push(mint_to_ix(&self.mint, &source.pubkey(), &self.setup.pubkey(), BUDGET));
            send(&self.rpc, &self.setup, &[&source], ixs)?;
            Ok((swig, wallet, source.pubkey()))
        };

        // Comparison wallet: an ordinary program allowlist for the agent's key.
        // Its root is the server key because it is only the control group.
        let (plain_swig, plain_wallet, plain_source) = fund(self.setup.pubkey(), 1)?;
        send(
            &self.rpc,
            &self.setup,
            &[],
            vec![add_plain_agent_ix(
                plain_swig,
                self.setup.pubkey(),
                self.setup.pubkey(),
                self.agent.pubkey(),
                self.target,
                self.mint,
            )?],
        )?;

        // Protected wallet: the operator is the Swig root. Swig's Create does
        // not need the root's signature, so the server can pay for it.
        let (guarded_swig, guarded_wallet, guarded_source) = fund(operator, 2)?;
        let policy = Keypair::new();
        let guarded = Guarded {
            gate: self.gate,
            swig: guarded_swig,
            wallet: guarded_wallet,
            policy: policy.pubkey(),
            pay: self.target,
        };
        let (phase, latest) = if operator == self.setup.pubkey() {
            send(
                &self.rpc,
                &self.setup,
                &[&policy],
                guarded.setup_ixs(
                    &self.rpc,
                    self.setup.pubkey(),
                    operator,
                    self.agent.pubkey(),
                    self.mint,
                    initial_slot,
                )?,
            )?;
            (
                Phase::Ready,
                json!({
                    "label": "Policy initialized",
                    "tone": "neutral",
                    "detail": format!("Veto policy {} approves merchant-pay at slot {}.", policy.pubkey(), initial_slot)
                }),
            )
        } else {
            (
                Phase::AwaitingApproval,
                json!({
                    "label": "Operator approval required",
                    "tone": "neutral",
                    "detail": format!(
                        "Operator {} must sign the setup: approve merchant-pay at slot {} and add the Veto-bound agent role.",
                        operator, initial_slot
                    )
                }),
            )
        };
        Ok(Run {
            token: new_token(),
            last_activity: Instant::now(),
            operator,
            policy,
            guarded,
            guarded_source,
            plain_swig,
            plain_wallet,
            plain_source,
            phase,
            latest,
            pending_approval: None,
        })
    }
}

impl DemoState {
    fn initialize() -> Result<Self> {
        let shared = Shared::initialize()?;
        let mut state = Self {
            shared,
            run: None,
            run_starts: VecDeque::new(),
        };
        // Local modes start one run immediately for the configured operator.
        if !state.shared.hosted {
            let operator = env::var("OPERATOR_PUBKEY")
                .ok()
                .map(|value| value.parse().context("invalid OPERATOR_PUBKEY"))
                .transpose()?
                .unwrap_or_else(|| state.shared.setup.pubkey());
            state.run = Some(state.shared.start_run(operator)?);
        }
        Ok(state)
    }

    fn run_is_free(&self, caller: Option<&str>) -> bool {
        match &self.run {
            None => true,
            Some(run) => {
                run.phase == Phase::Resumed
                    || run.last_activity.elapsed() > RUN_IDLE_TIMEOUT
                    || caller == Some(run.token.as_str())
            },
        }
    }

    /// Starts a hosted run. `operator` is the visitor's wallet address, or
    /// `None` for the server's demo operator key.
    fn start(&mut self, operator: Option<Pubkey>, caller: Option<&str>) -> Result<String> {
        if !self.run_is_free(caller) {
            bail!("another visitor is running the demo; try again in a few minutes")
        }
        let hour = Duration::from_secs(3600);
        while self.run_starts.front().is_some_and(|start| start.elapsed() > hour) {
            self.run_starts.pop_front();
        }
        if self.run_starts.len() >= self.shared.max_runs_per_hour {
            bail!("the hourly demo limit has been reached; try again later")
        }
        if self.shared.rpc.get_balance(&self.shared.setup.pubkey())? < MIN_SETUP_LAMPORTS {
            bail!("the demo's devnet fee payer is low on test SOL; try again later")
        }
        let operator = operator.unwrap_or_else(|| self.shared.setup.pubkey());
        let run = self.shared.start_run(operator)?;
        let token = run.token.clone();
        self.run = Some(run);
        self.run_starts.push_back(Instant::now());
        Ok(token)
    }

    fn status(&self, caller: Option<&str>) -> Result<Value> {
        let shared = &self.shared;
        let (_, current_slot) = programdata_and_slot(&shared.rpc, &shared.target)?;
        let current_hash = deployed_code_hash(&shared.rpc, &shared.target)?;
        let mut status = json!({
            "hosted": shared.hosted,
            "rpcUrl": shared.rpc_url,
            "target": shared.target.to_string(),
            "agent": shared.agent.pubkey().to_string(),
            "mint": shared.mint.to_string(),
            "currentSlot": current_slot,
            "currentCodeHash": current_hash,
            "reviewedCodeHash": shared.reviewed_hash,
            "codeMatchesReviewed": current_hash == shared.reviewed_hash,
            "budget": units(BUDGET),
            "payment": units(PAYMENT),
            "canStart": self.run_is_free(caller),
        });
        let Some(run) = &self.run else {
            status["phase"] = json!("idle");
            status["owned"] = json!(false);
            status["approvedSlot"] = Value::Null;
            status["latest"] = json!({ "label": "No run yet", "tone": "neutral", "detail": "" });
            return Ok(status);
        };
        let owned = !shared.hosted || caller == Some(run.token.as_str());
        let fields = json!({
            "phase": run.phase.as_str(),
            "owned": owned,
            "swigConfig": run.guarded.swig.to_string(),
            "swigWallet": run.guarded.wallet.to_string(),
            "plainSwigConfig": run.plain_swig.to_string(),
            "plainWallet": run.plain_wallet.to_string(),
            "policy": run.policy.pubkey().to_string(),
            "operator": run.operator.to_string(),
            "swigRoot": run.operator.to_string(),
            "externalOperator": run.operator != shared.setup.pubkey(),
            "approvedSlot": run.guarded.approved_slot(&shared.rpc)?,
            "balances": {
                "veto": units(balance(&shared.rpc, &run.guarded_source)?),
                "plain": units(balance(&shared.rpc, &run.plain_source)?),
                "merchant": units(balance(&shared.rpc, &shared.merchant)?),
            },
            "latest": &run.latest,
        });
        for (key, value) in fields.as_object().into_iter().flatten() {
            status[key] = value.clone();
        }
        Ok(status)
    }

    /// The current run, if the caller may drive it. Locally every caller may.
    fn owned_run(&mut self, caller: Option<&str>) -> Result<&mut Run> {
        let hosted = self.shared.hosted;
        let run = self.run.as_mut().ok_or_else(|| anyhow!("start a run first"))?;
        if hosted && caller != Some(run.token.as_str()) {
            bail!("this run belongs to another visitor")
        }
        run.last_activity = Instant::now();
        Ok(run)
    }

    fn prepare_operator_approval(&mut self, caller: Option<&str>) -> Result<Value> {
        self.owned_run(caller)?;
        let shared = &self.shared;
        let run = self.run.as_mut().ok_or_else(|| anyhow!("start a run first"))?;
        let (_, reviewed_slot) = programdata_and_slot(&shared.rpc, &shared.target)?;
        let (instructions, initializes_policy) = match run.phase {
            Phase::AwaitingApproval => (
                run.guarded.setup_ixs(
                    &shared.rpc,
                    shared.setup.pubkey(),
                    run.operator,
                    shared.agent.pubkey(),
                    shared.mint,
                    reviewed_slot,
                )?,
                true,
            ),
            Phase::Fixed => (
                vec![run
                    .guarded
                    .approval_ix(1, run.operator, shared.agent.pubkey(), reviewed_slot)],
                false,
            ),
            Phase::Upgraded | Phase::Blocked => bail!(
                "the deployed code does not match the reviewed build; this console will not prepare an approval for it"
            ),
            _ => bail!(
                "operator approval is not valid at phase {}",
                run.phase.as_str()
            ),
        };
        let blockhash = shared.rpc.get_latest_blockhash()?;
        let mut transaction =
            Transaction::new_unsigned(Message::new(&instructions, Some(&shared.setup.pubkey())));
        transaction.try_partial_sign(&[&shared.setup], blockhash)?;
        if initializes_policy {
            transaction.try_partial_sign(&[&run.policy], blockhash)?;
        }
        let encoded = BASE64.encode(bincode::serialize(&transaction)?);
        run.pending_approval = Some(PendingApproval {
            transaction,
            reviewed_slot,
            initializes_policy,
        });
        Ok(json!({
            "transaction": encoded,
            "operator": run.operator.to_string(),
            "target": shared.target.to_string(),
            "policy": run.policy.pubkey().to_string(),
            "reviewedSlot": reviewed_slot,
            "kind": if initializes_policy { "initialize" } else { "reapprove" }
        }))
    }

    fn submit_operator_approval(&mut self, encoded: &str, caller: Option<&str>) -> Result<Value> {
        self.owned_run(caller)?;
        let shared = &self.shared;
        let run = self.run.as_mut().ok_or_else(|| anyhow!("start a run first"))?;
        let pending = run
            .pending_approval
            .clone()
            .ok_or_else(|| anyhow!("no operator approval transaction is pending"))?;
        let bytes = BASE64
            .decode(encoded)
            .context("operator transaction is not valid base64")?;
        if bytes.len() > MAX_BODY_BYTES {
            bail!("operator transaction exceeds the size limit")
        }
        let transaction: Transaction =
            bincode::deserialize(&bytes).context("operator transaction is malformed")?;
        if transaction.message != pending.transaction.message {
            bail!("operator changed the reviewed approval transaction")
        }
        for (expected, actual) in pending
            .transaction
            .signatures
            .iter()
            .zip(transaction.signatures.iter())
        {
            if *expected != Signature::default() && expected != actual {
                bail!("operator transaction discarded or replaced the sponsor signature")
            }
        }
        transaction
            .verify()
            .context("operator transaction is missing a valid authority signature")?;
        let signature = shared.rpc.send_and_confirm_transaction(&transaction)?;
        if run.guarded.approved_slot(&shared.rpc)? != Some(pending.reviewed_slot) {
            bail!("operator approval did not pin the reviewed deployment slot")
        }
        run.phase = if pending.initializes_policy {
            Phase::Ready
        } else {
            Phase::Reapproved
        };
        run.latest = json!({
            "label": if pending.initializes_policy { "Operator approval recorded" } else { "Reviewed build approved" },
            "tone": "success",
            "detail": format!(
                "Operator {} approved merchant-pay at slot {}. Signature: {}.",
                run.operator, pending.reviewed_slot, signature
            )
        });
        run.pending_approval = None;
        self.status(caller)
    }

    fn act(&mut self, action: &str, caller: Option<&str>) -> Result<Value> {
        self.owned_run(caller)?;
        let shared = &self.shared;
        let run = self.run.as_mut().ok_or_else(|| anyhow!("start a run first"))?;
        let rpc = &shared.rpc;
        let agent = &shared.agent;
        let balances = |run: &Run| -> Result<(u64, u64)> {
            Ok((balance(rpc, &run.guarded_source)?, balance(rpc, &run.plain_source)?))
        };
        let plain_pay = |run: &Run| -> Result<Signature> {
            send(
                rpc,
                agent,
                &[],
                vec![plain_payment(
                    run.plain_swig,
                    run.plain_wallet,
                    agent.pubkey(),
                    pay_ix(shared.target, run.plain_source, shared.merchant, run.plain_wallet, PAYMENT),
                )?],
            )
        };
        let guarded_pay = |run: &Run| {
            run.guarded.payment(
                agent.pubkey(),
                pay_ix(shared.target, run.guarded_source, shared.merchant, run.guarded.wallet, PAYMENT),
            )
        };
        match (action, run.phase) {
            ("execute", Phase::Ready) => {
                let (veto_before, plain_before) = balances(run)?;
                let plain_signature = plain_pay(run)?;
                let signature = send(rpc, agent, &[], guarded_pay(run)?)?;
                let (veto, plain) = balances(run)?;
                if veto_before - veto != PAYMENT || plain_before - plain != PAYMENT {
                    bail!("reviewed payments did not charge exactly the requested amount")
                }
                run.phase = Phase::Executed;
                run.latest = json!({
                    "label": "Both wallets paid under the reviewed build",
                    "tone": "success",
                    "detail": format!(
                        "The agent paid {} from each wallet. Veto wallet {} → {}. Signature: {}. Plain allowlist signature: {}.",
                        units(PAYMENT), units(veto_before), units(veto), signature, plain_signature
                    )
                });
            },
            ("upgrade", Phase::Executed) => {
                let (old_slot, new_slot, signature) = shared.upgrade_to(&shared.pay_v2)?;
                run.phase = Phase::Upgraded;
                run.latest = json!({
                    "label": "Protocol shipped v2",
                    "tone": "warning",
                    "detail": format!(
                        "The protocol's upgrade key replaced merchant-pay under the same program ID. Slot {} → {}. The deployed code no longer matches the reviewed build. Signature: {}.",
                        old_slot, new_slot, signature
                    )
                });
            },
            ("probe", Phase::Upgraded) => {
                let (veto_before, plain_before) = balances(run)?;
                let plain_signature = plain_pay(run)?;
                let (signature, error) = send_expected_failure(rpc, agent, guarded_pay(run)?, 4)?;
                let (veto, plain) = balances(run)?;
                if veto != veto_before {
                    bail!("the blocked Veto payment changed the wallet balance")
                }
                run.phase = Phase::Blocked;
                run.latest = json!({
                    "label": "Veto blocked the payment",
                    "tone": "warning",
                    "detail": format!(
                        "The agent asked for {} from each wallet. Plain allowlist was charged {}. Veto rejected the call ({}), balance unchanged at {}. Signature: {}. Plain allowlist signature: {}.",
                        units(PAYMENT), units(plain_before - plain), error, units(veto), signature, plain_signature
                    )
                });
            },
            ("fix", Phase::Blocked) => {
                let (old_slot, new_slot, signature) = shared.upgrade_to(&shared.pay_v1)?;
                run.phase = Phase::Fixed;
                run.latest = json!({
                    "label": "Protocol redeployed the reviewed build",
                    "tone": "neutral",
                    "detail": format!(
                        "Slot {} → {}. The deployed code matches the reviewed build again, but Veto keeps the payment paused until the operator approves this deployment. Signature: {}.",
                        old_slot, new_slot, signature
                    )
                });
            },
            ("reapprove", Phase::Fixed) if run.operator == shared.setup.pubkey() => {
                let (_, current_slot) = programdata_and_slot(rpc, &shared.target)?;
                let signature = send(
                    rpc,
                    &shared.setup,
                    &[],
                    vec![run.guarded.approval_ix(1, run.operator, agent.pubkey(), current_slot)],
                )?;
                if run.guarded.approved_slot(rpc)? != Some(current_slot) {
                    bail!("operator reapproval did not pin the current deployment")
                }
                run.phase = Phase::Reapproved;
                run.latest = json!({
                    "label": "Reviewed build approved",
                    "tone": "success",
                    "detail": format!("Operator approved merchant-pay at slot {}. Signature: {}.", current_slot, signature)
                });
            },
            ("reapprove", Phase::Fixed) => bail!("external operator signature required"),
            ("resume", Phase::Reapproved) => {
                let (veto_before, _) = balances(run)?;
                let signature = send(rpc, agent, &[], guarded_pay(run)?)?;
                let (veto, _) = balances(run)?;
                if veto_before - veto != PAYMENT {
                    bail!("resumed payment did not charge exactly the requested amount")
                }
                run.phase = Phase::Resumed;
                run.latest = json!({
                    "label": "Payments resumed",
                    "tone": "success",
                    "detail": format!(
                        "The agent paid {} under the approved deployment. Veto wallet {} → {}. Signature: {}.",
                        units(PAYMENT), units(veto_before), units(veto), signature
                    )
                });
            },
            _ => bail!(
                "action {action} is not valid at phase {}",
                run.phase.as_str()
            ),
        }
        self.status(caller)
    }
}

struct Request {
    method: String,
    path: String,
    host: String,
    origin: Option<String>,
    cookie: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut bytes = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    while bytes.len() < MAX_HEADER_BYTES && !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            bail!("connection closed before request headers")
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .ok_or_else(|| anyhow!("request headers exceed the size limit"))?;
    let request = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = request.split("\r\n");
    let mut request_line = lines
        .next()
        .ok_or_else(|| anyhow!("missing request line"))?
        .split_whitespace();
    let method = request_line
        .next()
        .ok_or_else(|| anyhow!("missing HTTP method"))?
        .to_owned();
    let path = request_line
        .next()
        .ok_or_else(|| anyhow!("missing request path"))?
        .split('?')
        .next()
        .unwrap_or("/")
        .to_owned();
    let mut host = None;
    let mut origin = None;
    let mut cookie = None;
    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => host = Some(value.trim().to_owned()),
                "origin" => origin = Some(value.trim().to_owned()),
                "cookie" => {
                    cookie = value.split(';').find_map(|pair| {
                        let (key, value) = pair.trim().split_once('=')?;
                        (key == COOKIE).then(|| value.to_owned())
                    })
                },
                "content-length" => {
                    content_length = value.trim().parse().context("invalid Content-Length")?;
                },
                _ => {},
            }
        }
    }
    if content_length > MAX_BODY_BYTES {
        bail!("request body exceeds the size limit")
    }
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            bail!("connection closed before request body")
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    let body = bytes[header_end..header_end + content_length].to_vec();
    Ok(Request {
        method,
        path,
        host: host.unwrap_or_default(),
        origin,
        cookie,
        body,
    })
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
    extra_headers: &str,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Response",
    };
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; script-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; frame-ancestors 'none'\r\n{extra_headers}\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn json_response(stream: &mut TcpStream, result: Result<Value>, extra_headers: &str) -> Result<()> {
    match result {
        Ok(value) => write_response(
            stream,
            200,
            "application/json; charset=utf-8",
            &value.to_string(),
            extra_headers,
        ),
        Err(error) => write_response(
            stream,
            409,
            "application/json; charset=utf-8",
            &json!({ "error": error.to_string() }).to_string(),
            "",
        ),
    }
}

struct HttpConfig {
    allowed_hosts: Vec<String>,
    allowed_origins: Vec<String>,
    secure_cookie: bool,
}

/// Serves one request. Status reads fall back to the last computed status
/// while a slow transaction holds the state lock, so one visitor's devnet
/// upgrade does not freeze everyone else's page.
fn handle(
    mut stream: TcpStream,
    state: &Mutex<DemoState>,
    cached: &Mutex<Value>,
    config: &HttpConfig,
) -> Result<()> {
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            let body = json!({ "error": error.to_string() }).to_string();
            let _ = write_response(&mut stream, 400, "application/json; charset=utf-8", &body, "");
            return Ok(());
        },
    };
    if !config.allowed_hosts.contains(&request.host) {
        return write_response(&mut stream, 403, "text/plain; charset=utf-8", "Unknown host", "");
    }
    if request.method == "POST"
        && !config
            .allowed_origins
            .iter()
            .any(|origin| Some(origin.as_str()) == request.origin.as_deref())
    {
        return write_response(
            &mut stream,
            403,
            "text/plain; charset=utf-8",
            "Same-origin request required",
            "",
        );
    }
    let caller = request.cookie.as_deref();
    // Actions refresh the cached status so other visitors see current state
    // while a later slow action holds the lock.
    let remember = |result: Result<Value>| -> Result<Value> {
        if let (Ok(status), Ok(mut cache)) = (&result, cached.lock()) {
            *cache = status.clone();
        }
        result
    };
    let busy = || anyhow!("another action is in progress; try again in a moment");
    let lock = || match state.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::WouldBlock) => None,
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
    };

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => write_response(&mut stream, 200, "text/html; charset=utf-8", INDEX_HTML, ""),
        ("GET", "/favicon.svg") => {
            write_response(&mut stream, 200, "image/svg+xml", FAVICON_SVG, "")
        },
        ("GET", "/tokens.css") => {
            write_response(&mut stream, 200, "text/css; charset=utf-8", TOKENS_CSS, "")
        },
        ("GET", "/wallet-client.js") => write_response(
            &mut stream,
            200,
            "text/javascript; charset=utf-8",
            WALLET_CLIENT_JS,
            "",
        ),
        ("GET", "/api/status") => {
            let result = match lock() {
                Some(guard) => {
                    let result = guard.status(caller);
                    if let (Ok(status), Ok(mut cache)) = (&result, cached.lock()) {
                        *cache = status.clone();
                    }
                    result
                },
                None => {
                    let mut status =
                        cached.lock().map(|cache| cache.clone()).unwrap_or(Value::Null);
                    if status.is_null() {
                        Err(busy())
                    } else {
                        status["busy"] = json!(true);
                        status["owned"] = json!(false);
                        Ok(status)
                    }
                },
            };
            match result {
                Ok(status) => json_response(&mut stream, Ok(status), ""),
                Err(error) => write_response(
                    &mut stream,
                    500,
                    "application/json; charset=utf-8",
                    &json!({ "error": error.to_string() }).to_string(),
                    "",
                ),
            }
        },
        ("POST", "/api/run") => {
            let Some(mut guard) = lock() else {
                return json_response(&mut stream, Err(busy()), "");
            };
            if !guard.shared.hosted {
                return json_response(
                    &mut stream,
                    Err(anyhow!("runs start automatically in local mode")),
                    "",
                );
            }
            let operator = serde_json::from_slice::<Value>(&request.body)
                .context("run request is not valid JSON")
                .and_then(|payload| match payload.get("operator").and_then(Value::as_str) {
                    None | Some("demo") => Ok(None),
                    Some(address) => address
                        .parse::<Pubkey>()
                        .map(Some)
                        .context("operator is not a valid Solana address"),
                });
            match operator.and_then(|operator| guard.start(operator, caller)) {
                Ok(token) => {
                    let secure = if config.secure_cookie { "; Secure" } else { "" };
                    let cookie = format!(
                        "Set-Cookie: {COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict{secure}\r\n"
                    );
                    json_response(&mut stream, remember(guard.status(Some(&token))), &cookie)
                },
                Err(error) => json_response(&mut stream, Err(error), ""),
            }
        },
        ("POST", "/api/operator/approval-transaction") => {
            let Some(mut guard) = lock() else {
                return json_response(&mut stream, Err(busy()), "");
            };
            json_response(&mut stream, guard.prepare_operator_approval(caller), "")
        },
        ("POST", "/api/operator/submit-approval") => {
            let Some(mut guard) = lock() else {
                return json_response(&mut stream, Err(busy()), "");
            };
            let result = serde_json::from_slice::<Value>(&request.body)
                .context("approval submission is not valid JSON")
                .and_then(|payload| {
                    payload
                        .get("transaction")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or_else(|| anyhow!("approval submission is missing transaction"))
                })
                .and_then(|transaction| guard.submit_operator_approval(&transaction, caller));
            json_response(&mut stream, remember(result), "")
        },
        ("POST", path) if path.starts_with("/api/action/") => {
            let Some(mut guard) = lock() else {
                return json_response(&mut stream, Err(busy()), "");
            };
            let action = path.trim_start_matches("/api/action/");
            json_response(&mut stream, remember(guard.act(action, caller)), "")
        },
        ("GET", _) | ("POST", _) => write_response(
            &mut stream,
            404,
            "application/json; charset=utf-8",
            &json!({ "error": "route not found" }).to_string(),
            "",
        ),
        _ => write_response(
            &mut stream,
            405,
            "application/json; charset=utf-8",
            &json!({ "error": "method not allowed" }).to_string(),
            "",
        ),
    }
}

fn main() -> Result<()> {
    let state = DemoState::initialize().context("could not initialize the Veto operator demo")?;
    let bind_addr = state.shared.bind_addr.clone();
    let port = bind_addr.rsplit(':').next().unwrap_or("4173").to_owned();
    let mut allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let mut allowed_origins = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];
    let public_origin = state.shared.public_origin.clone();
    if let Some(origin) = &public_origin {
        let origin = origin.trim_end_matches('/');
        allowed_origins.push(origin.to_owned());
        if let Some(host) = origin.split("://").nth(1) {
            allowed_hosts.push(host.to_owned());
        }
    }
    let config = Arc::new(HttpConfig {
        allowed_hosts,
        allowed_origins,
        secure_cookie: public_origin.is_some_and(|origin| origin.starts_with("https://")),
    });
    match &state.run {
        Some(run) => println!(
            "Veto operator UI ready at http://{bind_addr}; target={} policy={}",
            state.shared.target,
            run.policy.pubkey()
        ),
        None => println!(
            "Veto operator UI ready at http://{bind_addr} (hosted); target={}",
            state.shared.target
        ),
    }
    let state = Arc::new(Mutex::new(state));
    let cached = Arc::new(Mutex::new(Value::Null));
    let listener = TcpListener::bind(&bind_addr).context("could not bind the operator port")?;
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let (state, cached, config) = (state.clone(), cached.clone(), config.clone());
                thread::spawn(move || {
                    if let Err(error) = handle(stream, &state, &cached, &config) {
                        eprintln!("operator request failed: {error:#}");
                    }
                });
            },
            Err(error) => eprintln!("accept failed: {error}"),
        }
    }
    Ok(())
}
