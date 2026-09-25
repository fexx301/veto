//! Route-pinning demo: the agent's wallet may call `payment-router`, which
//! forwards payments to `merchant-pay`. Only the downstream merchant-pay is
//! upgraded. A plain program allowlist on the router keeps paying; Veto checks
//! every program the delegated call can reach and stops until the operator
//! approves the new merchant-pay deployment.

#[path = "../demo/payment.rs"]
mod payment;

use {
    anyhow::{anyhow, bail, Context, Result},
    payment::*,
    solana_client::rpc_client::RpcClient,
    solana_commitment_config::CommitmentConfig,
    solana_sdk::{
        instruction::{AccountMeta, Instruction},
        pubkey::Pubkey,
        signature::{read_keypair_file, Keypair},
        signer::Signer,
    },
    solana_system_interface::instruction as system_instruction,
    std::{env, process::Command},
};

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

fn router_ix(router: Pubkey, merchant_pay: Pubkey, source: Pubkey, merchant: Pubkey, wallet: Pubkey, amount: u64) -> Instruction {
    Instruction {
        program_id: router,
        accounts: vec![
            AccountMeta::new(source, false),
            AccountMeta::new(merchant, false),
            AccountMeta::new_readonly(wallet, true),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(merchant_pay, false),
        ],
        data: amount.to_le_bytes().to_vec(),
    }
}

/// Deploys `so` as a new program and removes its upgrade authority, as an
/// attacker could, to show that a finalized program is not implicitly trusted.
fn deploy_finalized(solana_bin: &str, rpc_url: &str, payer_path: &str, so: &str) -> Result<Pubkey> {
    let program = Keypair::new();
    let path = env::temp_dir().join(format!("veto-evil-{}.json", program.pubkey()));
    solana_sdk::signature::write_keypair_file(&program, &path).map_err(|error| anyhow!(error.to_string()))?;
    let run = |args: &[&str]| -> Result<()> {
        let output = Command::new(solana_bin).args(args).output().context("could not run solana")?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    };
    let path_str = path.to_string_lossy().to_string();
    run(&["program", "deploy", "--url", rpc_url, "--keypair", payer_path, "--program-id", &path_str, so])?;
    let id = program.pubkey().to_string();
    run(&["program", "set-upgrade-authority", "--url", rpc_url, "--keypair", payer_path, &id, "--final"])?;
    let _ = std::fs::remove_file(&path);
    Ok(program.pubkey())
}

fn main() -> Result<()> {
    let rpc_url = env::var("RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".into());
    let rpc = RpcClient::new_with_commitment(&rpc_url, CommitmentConfig::confirmed());
    let solana_bin = env::var("SOLANA_BIN").unwrap_or_else(|_| "solana".into());
    let setup = required_keypair("HUMAN_PATH")?;
    let agent = required_keypair("AGENT_PATH")?;
    let protocol_path = env::var("PROTOCOL_PATH").context("missing PROTOCOL_PATH")?;
    let protocol = read_keypair_file(&protocol_path).map_err(|error| anyhow!(error.to_string()))?;
    let gate = required_pubkey("GATE_ID")?;
    let router = required_pubkey("ROUTER_ID")?;
    let merchant_pay = required_pubkey("PAY_ID")?;
    let pay_keypair = env::var("PAY_KEYPAIR").context("missing PAY_KEYPAIR")?;
    let pay_v1 = env::var("PAY_V1_SO").context("missing PAY_V1_SO")?;
    let pay_v2 = env::var("PAY_V2_SO").context("missing PAY_V2_SO")?;
    let upgrade = |so: &str| deploy(&solana_bin, &rpc_url, &protocol_path, &pay_keypair, so);

    let operator = Keypair::new();
    send(
        &rpc,
        &setup,
        &[],
        vec![
            system_instruction::transfer(&setup.pubkey(), &operator.pubkey(), 1_000_000_000),
            system_instruction::transfer(&setup.pubkey(), &protocol.pubkey(), 5_000_000_000),
        ],
    )?;
    // The runtime rule route pinning relies on: a program cannot invoke a
    // program that is not among its instruction accounts, even when that
    // program appears elsewhere in the same transaction.
    let sneaky = required_pubkey("SNEAKY_ID")?;
    let mut hidden_call = vec![1];
    hidden_call.extend_from_slice(merchant_pay.as_ref());
    let placement = Instruction { program_id: sneaky, accounts: vec![AccountMeta::new_readonly(merchant_pay, false)], data: vec![0] };
    let hidden = Instruction { program_id: sneaky, accounts: vec![], data: hidden_call.clone() };
    let error = send_failure(&rpc, &setup, vec![placement, hidden])?;
    if !error.contains("InstructionError(1, MissingAccount)") {
        bail!("expected the runtime to reject an unlisted CPI callee, got {error}")
    }
    // Control: the same call with the callee passed gets past that check and
    // fails inside merchant-pay instead (no accounts to pay from).
    let listed = Instruction { program_id: sneaky, accounts: vec![AccountMeta::new_readonly(merchant_pay, false)], data: hidden_call };
    let control = send_failure(&rpc, &setup, vec![listed])?;
    if control.contains("MissingAccount") {
        bail!("control CPI with the callee listed was rejected as missing: {control}")
    }
    println!("runtime-rejects-unlisted-cpi-callee error={error} control-with-callee-listed={control}");

    let (_, router_slot) = programdata_and_slot(&rpc, &router)?;
    let (_, pay_slot) = programdata_and_slot(&rpc, &merchant_pay)?;
    println!("payment-router slot={router_slot}; merchant-pay (downstream) slot={pay_slot}");

    let mint = Keypair::new();
    let merchant_owner = Keypair::new();
    let merchant = Keypair::new();
    let mut ixs = create_mint_ixs(&rpc, &setup.pubkey(), &mint.pubkey(), &setup.pubkey())?;
    ixs.extend(create_token_account_ixs(&rpc, &setup.pubkey(), &merchant.pubkey(), &mint.pubkey(), &merchant_owner.pubkey())?);
    send(&rpc, &setup, &[&mint, &merchant], ixs)?;

    let mut wallets = Vec::new();
    for id in [[41u8; 32], [42u8; 32]] {
        let (swig, wallet, create) = swig_create(id, setup.pubkey(), operator.pubkey())?;
        let source = Keypair::new();
        let mut ixs = vec![create];
        ixs.extend(create_token_account_ixs(&rpc, &setup.pubkey(), &source.pubkey(), &mint.pubkey(), &wallet)?);
        ixs.push(mint_to_ix(&mint.pubkey(), &source.pubkey(), &setup.pubkey(), BUDGET));
        send(&rpc, &setup, &[&source], ixs)?;
        wallets.push((swig, wallet, source));
    }
    let (plain_swig, plain_wallet, plain_source) = &wallets[0];
    let (guarded_swig, guarded_wallet, guarded_source) = &wallets[1];

    // Both wallets allow the agent to call the router only (first hop).
    send(
        &rpc,
        &setup,
        &[&operator],
        vec![add_plain_agent_ix(*plain_swig, setup.pubkey(), operator.pubkey(), agent.pubkey(), router, mint.pubkey())?],
    )?;
    let policy = Keypair::new();
    let guarded = Guarded { gate, swig: *guarded_swig, wallet: *guarded_wallet, policy: policy.pubkey(), pay: router };
    send(
        &rpc,
        &setup,
        &[&operator, &policy],
        guarded.setup_ixs(&rpc, setup.pubkey(), operator.pubkey(), agent.pubkey(), mint.pubkey(), router_slot)?,
    )?;

    let report = |label: &str| -> Result<()> {
        println!(
            "{label} plain={} veto={} merchant={}",
            units(balance(&rpc, &plain_source.pubkey())?),
            units(balance(&rpc, &guarded_source.pubkey())?),
            units(balance(&rpc, &merchant.pubkey())?)
        );
        Ok(())
    };
    let plain_pay = || {
        plain_payment(
            *plain_swig,
            *plain_wallet,
            agent.pubkey(),
            router_ix(router, merchant_pay, plain_source.pubkey(), merchant.pubkey(), *plain_wallet, PAYMENT),
        )
    };
    let inner = || router_ix(router, merchant_pay, guarded_source.pubkey(), merchant.pubkey(), *guarded_wallet, PAYMENT);
    let guarded_pay = || guarded.payment_via(agent.pubkey(), inner(), &[merchant_pay]);

    // The operator has approved the router but not yet merchant-pay: an
    // unreviewed downstream program is refused.
    send_expected_failure(&rpc, &agent, guarded_pay()?, 19)?;
    println!("unapproved-downstream-rejected custom=19");
    // Only the operator can approve a downstream program.
    send_expected_failure(&rpc, &agent, vec![guarded.approve_program_ix(agent.pubkey(), merchant_pay, pay_slot)], 2)?;
    send(&rpc, &setup, &[&operator], vec![guarded.approve_program_ix(operator.pubkey(), merchant_pay, pay_slot)])?;
    println!("agent-approval-rejected custom=2 operator-approved merchant-pay slot={pay_slot}");
    // Veto must see the downstream program's ProgramData to check it.
    send_expected_failure(&rpc, &agent, guarded.payment_via(agent.pubkey(), inner(), &[])?, 18)?;
    println!("missing-downstream-programdata-rejected custom=18");

    // A malicious program that nobody can upgrade is still not approved.
    let human_path = env::var("HUMAN_PATH").context("missing HUMAN_PATH")?;
    let evil = deploy_finalized(&solana_bin, &rpc_url, &human_path, &pay_v2)?;
    let (_, evil_slot) = programdata_and_slot(&rpc, &evil)?;
    wait_past_slot(&rpc, evil_slot)?;
    let before = balance(&rpc, &guarded_source.pubkey())?;
    let evil_route = router_ix(router, evil, guarded_source.pubkey(), merchant.pubkey(), *guarded_wallet, PAYMENT);
    send_expected_failure(&rpc, &agent, guarded.payment_via(agent.pubkey(), evil_route, &[evil])?, 19)?;
    if balance(&rpc, &guarded_source.pubkey())? != before {
        bail!("route through a finalized malicious program moved funds")
    }
    println!("finalized-unapproved-program-rejected custom=19 program={evil}");

    // 1. Both wallets pay 10 through the router under the reviewed builds.
    send(&rpc, &agent, &[], vec![plain_pay()?])?;
    send(&rpc, &agent, &[], guarded_pay()?)?;
    if balance(&rpc, &plain_source.pubkey())? != BUDGET - PAYMENT || balance(&rpc, &guarded_source.pubkey())? != BUDGET - PAYMENT {
        bail!("reviewed payments did not charge exactly the requested amount")
    }
    report("v1-payments")?;

    // 2. Only the downstream merchant-pay is upgraded; the router is untouched.
    upgrade(&pay_v2)?;
    let (_, v2_slot) = programdata_and_slot(&rpc, &merchant_pay)?;
    wait_past_slot(&rpc, v2_slot)?;
    let (_, router_after) = programdata_and_slot(&rpc, &router)?;
    if router_after != router_slot || v2_slot == pay_slot {
        bail!("expected only merchant-pay to change")
    }
    println!("downstream-upgraded merchant-pay slot={pay_slot}->{v2_slot}; router slot unchanged={router_after}");

    // 3. Same request from both wallets.
    let before = balance(&rpc, &plain_source.pubkey())?;
    send(&rpc, &agent, &[], vec![plain_pay()?])?;
    let after = balance(&rpc, &plain_source.pubkey())?;
    println!("plain-allowlist(router) requested={} charged={}", units(PAYMENT), units(before - after));
    let guarded_before = balance(&rpc, &guarded_source.pubkey())?;
    let (signature, _) = send_expected_failure(&rpc, &agent, guarded_pay()?, 20)?;
    if balance(&rpc, &guarded_source.pubkey())? != guarded_before {
        bail!("blocked Veto payment changed the balance")
    }
    println!("veto-route-blocked custom=20 charged=0.00 signature={signature}");
    report("after-downstream-v2")?;

    // 4. The reviewed build is redeployed; Veto still waits for the operator.
    upgrade(&pay_v1)?;
    let (_, fixed_slot) = programdata_and_slot(&rpc, &merchant_pay)?;
    wait_past_slot(&rpc, fixed_slot)?;
    send_expected_failure(&rpc, &agent, guarded_pay()?, 20)?;
    println!("veto-still-blocked-until-review custom=20 merchant-pay slot={fixed_slot}");

    // 5. The operator approves the new downstream deployment; payments resume.
    send(&rpc, &setup, &[&operator], vec![guarded.approve_program_ix(operator.pubkey(), merchant_pay, fixed_slot)])?;
    send(&rpc, &agent, &[], guarded_pay()?)?;
    if balance(&rpc, &guarded_source.pubkey())? != BUDGET - 2 * PAYMENT {
        bail!("resumed payment did not charge exactly the requested amount")
    }
    report("resumed")?;
    println!("route-flow-passed");
    Ok(())
}
