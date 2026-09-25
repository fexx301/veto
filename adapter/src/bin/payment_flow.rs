//! Scripted test-token payment demo: two Swig wallets with the same budget
//! call the same upgradeable merchant-pay program. One wallet's agent role is
//! an ordinary program allowlist; the other's is bound to a Veto policy. The
//! program's upgrade authority then ships a v2 that charges the whole balance.

#[path = "../demo/payment.rs"]
mod payment;

use {
    anyhow::{anyhow, bail, Context, Result},
    payment::*,
    solana_client::rpc_client::RpcClient,
    solana_commitment_config::CommitmentConfig,
    solana_sdk::{
        pubkey::Pubkey,
        signature::{read_keypair_file, Keypair},
        signer::Signer,
    },
    solana_system_interface::instruction as system_instruction,
    std::env,
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

fn main() -> Result<()> {
    let rpc_url = env::var("RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".into());
    let rpc = RpcClient::new_with_commitment(&rpc_url, CommitmentConfig::confirmed());
    let solana_bin = env::var("SOLANA_BIN").unwrap_or_else(|_| "solana".into());
    let setup = required_keypair("HUMAN_PATH")?;
    let agent = required_keypair("AGENT_PATH")?;
    let protocol_path = env::var("PROTOCOL_PATH").context("missing PROTOCOL_PATH")?;
    let protocol = read_keypair_file(&protocol_path).map_err(|error| anyhow!(error.to_string()))?;
    let gate = required_pubkey("GATE_ID")?;
    let pay = required_pubkey("PAY_ID")?;
    let pay_keypair = env::var("PAY_KEYPAIR").context("missing PAY_KEYPAIR")?;
    let pay_v1 = env::var("PAY_V1_SO").context("missing PAY_V1_SO")?;
    let pay_v2 = env::var("PAY_V2_SO").context("missing PAY_V2_SO")?;
    let upgrade = |so: &str| deploy(&solana_bin, &rpc_url, &protocol_path, &pay_keypair, so);

    // The operator stands in for the human's browser wallet in this scripted
    // run: it is the Swig root of both wallets and the Veto approval authority.
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

    let v1_hash = file_code_hash(&pay_v1)?;
    let v2_hash = file_code_hash(&pay_v2)?;
    let (_, initial_slot) = programdata_and_slot(&rpc, &pay)?;
    if deployed_code_hash(&rpc, &pay)? != v1_hash {
        bail!("deployed merchant-pay is not the reviewed v1 build")
    }
    println!("merchant-pay v1 slot={initial_slot} code-sha256={v1_hash}");

    // Test-token mint, merchant account, and one funded account per wallet.
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

    let mut wallets = Vec::new();
    for id in [[31u8; 32], [32u8; 32]] {
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

    // Plain allowlist: the agent's key may call merchant-pay, up to the budget.
    send(
        &rpc,
        &setup,
        &[&operator],
        vec![add_plain_agent_ix(
            *plain_swig,
            setup.pubkey(),
            operator.pubkey(),
            agent.pubkey(),
            pay,
            mint.pubkey(),
        )?],
    )?;

    // Veto: the same permissions, but only through the operator's policy.
    let policy = Keypair::new();
    let guarded = Guarded {
        gate,
        swig: *guarded_swig,
        wallet: *guarded_wallet,
        policy: policy.pubkey(),
        pay,
    };
    send(
        &rpc,
        &setup,
        &[&operator, &policy],
        guarded.setup_ixs(&rpc, setup.pubkey(), operator.pubkey(), agent.pubkey(), mint.pubkey(), initial_slot)?,
    )?;
    println!(
        "setup budget={} plain-wallet={plain_wallet} veto-wallet={guarded_wallet} policy={}",
        units(BUDGET),
        policy.pubkey()
    );

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
            pay_ix(pay, plain_source.pubkey(), merchant.pubkey(), *plain_wallet, PAYMENT),
        )
    };
    let guarded_pay = || {
        guarded.payment(
            agent.pubkey(),
            pay_ix(pay, guarded_source.pubkey(), merchant.pubkey(), *guarded_wallet, PAYMENT),
        )
    };

    // 1. Both wallets pay 10 under the reviewed v1 code.
    send(&rpc, &agent, &[], vec![plain_pay()?])?;
    let signature = send(&rpc, &agent, &[], guarded_pay()?)?;
    if balance(&rpc, &plain_source.pubkey())? != BUDGET - PAYMENT
        || balance(&rpc, &guarded_source.pubkey())? != BUDGET - PAYMENT
    {
        bail!("v1 payments did not charge exactly the requested amount")
    }
    report("v1-payments")?;
    println!("veto-payment-signature={signature}");

    // 2. The program's upgrade authority ships v2 under the same program ID.
    upgrade(&pay_v2)?;
    let (_, v2_slot) = programdata_and_slot(&rpc, &pay)?;
    wait_past_slot(&rpc, v2_slot)?;
    if deployed_code_hash(&rpc, &pay)? != v2_hash || v2_slot == initial_slot {
        bail!("v2 upgrade did not change the deployed code and slot")
    }
    println!(
        "upgraded program-id={pay} slot={initial_slot}->{v2_slot} code-sha256={v2_hash} matches-reviewed-build=false"
    );

    // 3. The agent asks for the same 10-token payment from both wallets.
    let plain_before = balance(&rpc, &plain_source.pubkey())?;
    send(&rpc, &agent, &[], vec![plain_pay()?])?;
    let plain_after = balance(&rpc, &plain_source.pubkey())?;
    if plain_after != 0 {
        bail!("expected v2 to drain the plain-allowlist wallet")
    }
    println!(
        "plain-allowlist requested={} charged={}",
        units(PAYMENT),
        units(plain_before - plain_after)
    );
    let guarded_before = balance(&rpc, &guarded_source.pubkey())?;
    let (blocked, _) = send_expected_failure(&rpc, &agent, guarded_pay()?, 4)?;
    if balance(&rpc, &guarded_source.pubkey())? != guarded_before {
        bail!("blocked Veto payment changed the wallet balance")
    }
    println!("veto-blocked custom=4 charged=0.00 signature={blocked}");
    report("after-v2")?;

    // 4. The operator does not approve v2. The protocol ships a fix (the v1
    //    build again); the approval is still stale until the operator reviews it.
    upgrade(&pay_v1)?;
    let (_, fixed_slot) = programdata_and_slot(&rpc, &pay)?;
    wait_past_slot(&rpc, fixed_slot)?;
    let deployed = deployed_code_hash(&rpc, &pay)?;
    if deployed != v1_hash {
        bail!("fixed deployment does not match the reviewed v1 build")
    }
    println!("redeployed slot={fixed_slot} code-sha256={deployed} matches-reviewed-build=true");
    send_expected_failure(&rpc, &agent, guarded_pay()?, 4)?;
    println!("veto-still-blocked-until-review custom=4");

    // 5. Only the operator can approve the reviewed deployment slot.
    send_expected_failure(
        &rpc,
        &agent,
        vec![guarded.approval_ix(1, agent.pubkey(), agent.pubkey(), fixed_slot)],
        2,
    )?;
    send(
        &rpc,
        &setup,
        &[&operator],
        vec![guarded.approval_ix(1, operator.pubkey(), agent.pubkey(), fixed_slot)],
    )?;
    if guarded.approved_slot(&rpc)? != Some(fixed_slot) {
        bail!("operator reapproval did not pin the reviewed slot")
    }
    println!("agent-reapproval-rejected custom=2 operator-reapproved slot={fixed_slot}");
    send(&rpc, &agent, &[], guarded_pay()?)?;
    if balance(&rpc, &guarded_source.pubkey())? != BUDGET - 2 * PAYMENT {
        bail!("resumed Veto payment did not charge exactly the requested amount")
    }
    report("resumed")?;
    println!("payment-flow-passed");
    Ok(())
}
