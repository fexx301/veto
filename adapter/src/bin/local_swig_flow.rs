use {
    anyhow::{anyhow, bail, Context, Result},
    solana_client::{rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig},
    solana_commitment_config::CommitmentConfig,
    solana_loader_v3_interface::{get_program_data_address, state::UpgradeableLoaderState},
    solana_sdk::{
        instruction::{AccountMeta, Instruction},
        message::Message,
        pubkey::Pubkey,
        signature::{read_keypair_file, Keypair, Signature},
        signer::Signer,
        transaction::Transaction,
    },
    solana_system_interface::instruction as system_instruction,
    std::{env, process::Command, thread::sleep, time::Duration},
    swig_interface::{
        program_id as swig_program_id, swig_wallet_address, AddAuthorityInstruction,
        AuthorityConfig, ClientAction, CreateInstruction, SignV2Instruction,
    },
    swig_state::{
        action::{all::All, program::Program},
        authority::{programexec::ProgramExecAuthority, AuthorityType},
        swig::{swig_account_seeds, swig_wallet_address_seeds},
    },
    veto_swig_gate::{validate_next_swig_instruction, validate_swig_account_binding},
};

const POLICY_LEN: usize = 522;
const POLICY_SLOT_START: usize = 161;

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

fn send(
    rpc: &RpcClient,
    payer: &Keypair,
    extra: &[&Keypair],
    instructions: Vec<Instruction>,
) -> Result<Signature> {
    let blockhash = rpc.get_latest_blockhash()?;
    let mut signers: Vec<&dyn Signer> = vec![payer];
    for signer in extra {
        signers.push(*signer);
    }
    let tx = Transaction::new(
        &signers,
        Message::new(&instructions, Some(&payer.pubkey())),
        blockhash,
    );
    Ok(rpc.send_and_confirm_transaction(&tx)?)
}

fn send_expected_failure(
    rpc: &RpcClient,
    payer: &Keypair,
    instructions: Vec<Instruction>,
    expected_custom: u32,
) -> Result<()> {
    let blockhash = rpc.get_latest_blockhash()?;
    let tx = Transaction::new(
        &[payer],
        Message::new(&instructions, Some(&payer.pubkey())),
        blockhash,
    );
    let signature = rpc.send_transaction_with_config(
        &tx,
        RpcSendTransactionConfig {
            skip_preflight: true,
            ..RpcSendTransactionConfig::default()
        },
    )?;
    for _ in 0..40 {
        if let Some(status) = rpc.get_signature_status(&signature)? {
            let error = status
                .err()
                .ok_or_else(|| anyhow!("transaction unexpectedly succeeded"))?;
            let debug = format!("{error:?}");
            if !debug.contains(&format!("Custom({expected_custom})")) {
                bail!("expected Custom({expected_custom}), got {debug}");
            }
            return Ok(());
        }
        sleep(Duration::from_millis(150));
    }
    bail!("submitted transaction {signature} did not reach a status")
}

fn send_failure_any(
    rpc: &RpcClient,
    payer: &Keypair,
    instructions: Vec<Instruction>,
) -> Result<String> {
    let blockhash = rpc.get_latest_blockhash()?;
    let tx = Transaction::new(
        &[payer],
        Message::new(&instructions, Some(&payer.pubkey())),
        blockhash,
    );
    let signature = rpc.send_transaction_with_config(
        &tx,
        RpcSendTransactionConfig {
            skip_preflight: true,
            ..RpcSendTransactionConfig::default()
        },
    )?;
    for _ in 0..40 {
        if let Some(status) = rpc.get_signature_status(&signature)? {
            let error = status
                .err()
                .ok_or_else(|| anyhow!("transaction unexpectedly succeeded"))?;
            return Ok(format!("{error:?}"));
        }
        sleep(Duration::from_millis(150));
    }
    bail!("submitted transaction {signature} did not reach a status")
}

fn programdata_and_slot(rpc: &RpcClient, target: &Pubkey) -> Result<(Pubkey, u64)> {
    let programdata = get_program_data_address(target);
    let account = rpc
        .get_account(&programdata)
        .with_context(|| format!("missing ProgramData {programdata}"))?;
    let metadata_len = UpgradeableLoaderState::size_of_programdata_metadata();
    let metadata: UpgradeableLoaderState = bincode::deserialize(&account.data[..metadata_len])?;
    match metadata {
        UpgradeableLoaderState::ProgramData { slot, .. } => Ok((programdata, slot)),
        _ => bail!("canonical ProgramData has an unexpected loader state"),
    }
}

fn gate_ix(
    gate: Pubkey,
    swig: Pubkey,
    wallet: Pubkey,
    policy: Pubkey,
    target: Pubkey,
    programdata: Pubkey,
    agent: Pubkey,
) -> Instruction {
    let mut data = vec![2];
    data.extend_from_slice(policy.as_ref());
    Instruction {
        program_id: gate,
        accounts: vec![
            AccountMeta::new_readonly(swig, false),
            AccountMeta::new_readonly(wallet, false),
            AccountMeta::new_readonly(policy, false),
            AccountMeta::new_readonly(target, false),
            AccountMeta::new_readonly(programdata, false),
            AccountMeta::new_readonly(solana_sdk::sysvar::instructions::id(), false),
            AccountMeta::new_readonly(agent, true),
        ],
        data,
    }
}

fn initialize_ix(
    gate: Pubkey,
    policy: Pubkey,
    authority: Pubkey,
    target: Pubkey,
    programdata: Pubkey,
    swig_config: Pubkey,
    swig_wallet: Pubkey,
    agent: Pubkey,
    reviewed_slot: u64,
) -> Instruction {
    Instruction {
        program_id: gate,
        accounts: vec![
            AccountMeta::new(policy, true),
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new_readonly(target, false),
            AccountMeta::new_readonly(programdata, false),
            AccountMeta::new_readonly(swig_program_id(), false),
            AccountMeta::new_readonly(swig_config, false),
            AccountMeta::new_readonly(swig_wallet, false),
            AccountMeta::new_readonly(agent, false),
        ],
        data: approval_data(0, reviewed_slot),
    }
}

fn gated_sign(
    gate: Pubkey,
    swig: Pubkey,
    wallet: Pubkey,
    agent: Pubkey,
    policy: Pubkey,
    target: Pubkey,
    programdata: Pubkey,
    counter: Pubkey,
) -> Result<Vec<Instruction>> {
    let mut proof = gate_ix(gate, swig, wallet, policy, target, programdata, agent);
    // Veto reads every account the delegated call receives.
    proof.accounts.push(AccountMeta::new_readonly(counter, false));
    let inner = Instruction {
        program_id: target,
        accounts: vec![AccountMeta::new(counter, false)],
        data: vec![],
    };
    let instructions = SignV2Instruction::new_program_exec(swig, wallet, agent, proof, inner, 1)?;
    validate_next_swig_instruction(&instructions[1], &swig_program_id(), &target)
        .map_err(|error| anyhow!("adapter parser rejected generated payload: {error:?}"))?;
    validate_swig_account_binding(&instructions[1], &swig, &wallet)
        .map_err(|error| anyhow!("adapter wallet binding rejected generated payload: {error:?}"))?;
    Ok(instructions)
}

fn counter(rpc: &RpcClient, key: &Pubkey) -> Result<u64> {
    let account = rpc.get_account(key)?;
    Ok(u64::from_le_bytes(account.data[..8].try_into()?))
}

fn reapprove_ix(
    gate: Pubkey,
    human: Pubkey,
    policy: Pubkey,
    target: Pubkey,
    programdata: Pubkey,
    swig: Pubkey,
    swig_config: Pubkey,
    swig_wallet: Pubkey,
    reviewed_slot: u64,
) -> Instruction {
    Instruction {
        program_id: gate,
        accounts: vec![
            AccountMeta::new(policy, false),
            AccountMeta::new_readonly(human, true),
            AccountMeta::new_readonly(target, false),
            AccountMeta::new_readonly(programdata, false),
            AccountMeta::new_readonly(swig, false),
            AccountMeta::new_readonly(swig_config, false),
            AccountMeta::new_readonly(swig_wallet, false),
        ],
        data: approval_data(1, reviewed_slot),
    }
}

fn approval_data(discriminator: u8, reviewed_slot: u64) -> Vec<u8> {
    let mut data = vec![discriminator];
    data.extend_from_slice(&reviewed_slot.to_le_bytes());
    data
}

fn relay_swig_ix(relay: Pubkey, sign: Instruction) -> Result<Instruction> {
    if sign.accounts.len() != 6 {
        bail!("unexpected Swig SignV2 account shape for relay fixture")
    }
    let mut accounts = sign.accounts;
    accounts.push(AccountMeta::new_readonly(swig_program_id(), false));
    let mut data = b"relayswg".to_vec();
    data.extend_from_slice(&sign.data);
    Ok(Instruction {
        program_id: relay,
        accounts,
        data,
    })
}

fn deploy_target_upgrade(rpc_url: &str) -> Result<()> {
    let solana_bin = env::var("SOLANA_BIN").unwrap_or_else(|_| "solana".into());
    let human_path = env::var("HUMAN_PATH").context("missing HUMAN_PATH")?;
    let target_keypair = env::var("TARGET_KEYPAIR").context("missing TARGET_KEYPAIR")?;
    let target_so = env::var("TARGET_SO").context("missing TARGET_SO")?;
    let output = Command::new(solana_bin)
        .args([
            "program",
            "deploy",
            "--url",
            rpc_url,
            "--keypair",
            &human_path,
            "--program-id",
            &target_keypair,
            &target_so,
        ])
        .output()
        .context("could not run local Solana loader upgrade")?;
    if !output.status.success() {
        bail!(
            "local target upgrade failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    println!(
        "loader-upgrade={}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
    Ok(())
}

fn resume_after_upgrade(
    rpc: &RpcClient,
    human: &Keypair,
    agent: &Keypair,
    gate: Pubkey,
    target: Pubkey,
) -> Result<()> {
    let policy = required_pubkey("POLICY_ID")?;
    let counter_key = required_pubkey("COUNTER_ID")?;
    let swig = required_pubkey("SWIG_CONFIG_ID")?;
    let wallet = swig_wallet_address(&swig);
    let (programdata, upgraded_slot) = programdata_and_slot(rpc, &target)?;
    send(
        rpc,
        human,
        &[],
        vec![reapprove_ix(
            gate,
            human.pubkey(),
            policy,
            target,
            programdata,
            swig_program_id(),
            swig,
            wallet,
            upgraded_slot,
        )],
    )?;
    let policy_bytes = rpc.get_account_data(&policy)?;
    if u64::from_le_bytes(policy_bytes[POLICY_SLOT_START..POLICY_SLOT_START + 8].try_into()?)
        != upgraded_slot
    {
        bail!("human reapproval did not update the pinned slot")
    }
    send(
        rpc,
        agent,
        &[],
        gated_sign(
            gate,
            swig,
            wallet,
            agent.pubkey(),
            policy,
            target,
            programdata,
            counter_key,
        )?,
    )?;
    if counter(rpc, &counter_key)? != 2 {
        bail!("reapproved delegated target did not resume")
    }
    println!("human-reapproved slot={upgraded_slot} counter=2");
    Ok(())
}

fn main() -> Result<()> {
    let rpc_url = env::var("RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".into());
    let rpc = RpcClient::new_with_commitment(&rpc_url, CommitmentConfig::confirmed());
    let human = required_keypair("HUMAN_PATH")?;
    let agent = required_keypair("AGENT_PATH")?;
    let gate = required_pubkey("GATE_ID")?;
    let target = required_pubkey("TARGET_ID")?;
    let relay = required_pubkey("RELAY_ID")?;
    if env::var_os("RESUME_AFTER_UPGRADE").is_some() {
        return resume_after_upgrade(&rpc, &human, &agent, gate, target);
    }

    let id = [19u8; 32];
    let (swig, bump) = Pubkey::find_program_address(&swig_account_seeds(&id), &swig_program_id());
    let wallet = swig_wallet_address(&swig);
    let (derived_wallet, wallet_bump) = Pubkey::find_program_address(
        &swig_wallet_address_seeds(swig.as_ref()),
        &swig_program_id(),
    );
    if wallet != derived_wallet {
        bail!("Swig interface and state seed helpers disagree")
    }
    let second_id = [20u8; 32];
    let (second_swig, second_bump) =
        Pubkey::find_program_address(&swig_account_seeds(&second_id), &swig_program_id());
    let second_wallet = swig_wallet_address(&second_swig);
    let (_, second_wallet_bump) = Pubkey::find_program_address(
        &swig_wallet_address_seeds(second_swig.as_ref()),
        &swig_program_id(),
    );
    println!("swig-fixture program={} config={swig} config-bump={bump} wallet={wallet} wallet-bump={wallet_bump}", swig_program_id());
    let (programdata, initial_slot) = programdata_and_slot(&rpc, &target)?;

    let create_swig = CreateInstruction::new(
        swig,
        bump,
        human.pubkey(),
        wallet,
        wallet_bump,
        AuthorityConfig {
            authority_type: AuthorityType::Ed25519,
            authority: human.pubkey().as_ref(),
        },
        vec![ClientAction::All(All {})],
        id,
    )?;
    send(&rpc, &human, &[], vec![create_swig])?;

    let policy = Keypair::new();
    let counter_key = Keypair::new();
    let rent_policy = rpc.get_minimum_balance_for_rent_exemption(POLICY_LEN)?;
    let rent_counter = rpc.get_minimum_balance_for_rent_exemption(8)?;
    send(
        &rpc,
        &human,
        &[&counter_key],
        vec![system_instruction::create_account(
            &human.pubkey(),
            &counter_key.pubkey(),
            rent_counter,
            8,
            &target,
        )],
    )?;

    // Swig stores this complete prefix in the delegated role. An agent cannot
    // substitute a separately initialized policy account into later proofs.
    let mut role_prefix = vec![2];
    role_prefix.extend_from_slice(policy.pubkey().as_ref());
    let program_exec_data =
        ProgramExecAuthority::create_authority_data(&gate.to_bytes(), &role_prefix);
    let add_adapter = AddAuthorityInstruction::new_with_ed25519_authority(
        swig,
        human.pubkey(),
        human.pubkey(),
        0,
        AuthorityConfig {
            authority_type: AuthorityType::ProgramExec,
            authority: &program_exec_data,
        },
        vec![ClientAction::Program(Program {
            program_id: target.to_bytes(),
        })],
    )?;
    // Create, initialize, and bind the policy in one transaction so nobody can
    // initialize the account between its creation and the operator's approval.
    send(
        &rpc,
        &human,
        &[&policy],
        vec![
            system_instruction::create_account(
                &human.pubkey(),
                &policy.pubkey(),
                rent_policy,
                POLICY_LEN as u64,
                &gate,
            ),
            initialize_ix(
                gate,
                policy.pubkey(),
                human.pubkey(),
                target,
                programdata,
                swig,
                wallet,
                agent.pubkey(),
                initial_slot,
            ),
            add_adapter,
        ],
    )?;

    // An agent can create a policy-shaped record owned by the adapter, but
    // cannot substitute it for the policy key bound into the Swig role.
    let substitute_policy = Keypair::new();
    send(
        &rpc,
        &agent,
        &[&substitute_policy],
        vec![
            system_instruction::create_account(
                &agent.pubkey(),
                &substitute_policy.pubkey(),
                rent_policy,
                POLICY_LEN as u64,
                &gate,
            ),
            initialize_ix(
                gate,
                substitute_policy.pubkey(),
                agent.pubkey(),
                target,
                programdata,
                swig,
                wallet,
                agent.pubkey(),
                initial_slot,
            ),
        ],
    )?;
    let initial_policy_bytes = rpc.get_account_data(&policy.pubkey())?;
    if initial_policy_bytes[97..129] != swig.to_bytes()
        || initial_policy_bytes[129..161] != wallet.to_bytes()
        || initial_policy_bytes[169..201] != agent.pubkey().to_bytes()
    {
        bail!("policy did not persist the intended Swig config and wallet")
    }
    send_expected_failure(
        &rpc,
        &human,
        vec![reapprove_ix(
            gate,
            human.pubkey(),
            policy.pubkey(),
            target,
            programdata,
            swig_program_id(),
            second_swig,
            wallet,
            initial_slot,
        )],
        12,
    )?;
    send_expected_failure(
        &rpc,
        &human,
        vec![reapprove_ix(
            gate,
            human.pubkey(),
            policy.pubkey(),
            target,
            programdata,
            swig_program_id(),
            swig,
            second_wallet,
            initial_slot,
        )],
        13,
    )?;
    let policy_after_bad_reapprovals = rpc.get_account_data(&policy.pubkey())?;
    if u64::from_le_bytes(
        policy_after_bad_reapprovals[POLICY_SLOT_START..POLICY_SLOT_START + 8].try_into()?,
    ) != initial_slot
    {
        bail!("mismatched-wallet reapproval changed the pinned slot")
    }
    println!(
        "wrong-config-reapproval-rejected custom=12 wrong-wallet-reapproval-rejected custom=13"
    );
    let substituted_policy_attempt = send_failure_any(
        &rpc,
        &agent,
        gated_sign(
            gate,
            swig,
            wallet,
            agent.pubkey(),
            substitute_policy.pubkey(),
            target,
            programdata,
            counter_key.pubkey(),
        )?,
    )?;
    if counter(&rpc, &counter_key.pubkey())? != 0 {
        bail!("substituted policy executed the target")
    }
    println!("policy-substitution-rejected error={substituted_policy_attempt} counter=0");

    // The agent cannot replace the human signature to update the approved slot.
    send_expected_failure(
        &rpc,
        &agent,
        vec![reapprove_ix(
            gate,
            agent.pubkey(),
            policy.pubkey(),
            target,
            programdata,
            swig_program_id(),
            swig,
            wallet,
            initial_slot,
        )],
        2,
    )?;
    let policy_bytes = rpc.get_account_data(&policy.pubkey())?;
    if u64::from_le_bytes(policy_bytes[POLICY_SLOT_START..POLICY_SLOT_START + 8].try_into()?)
        != initial_slot
    {
        bail!("unauthorized agent reapproval changed the pinned slot")
    }
    println!("agent-reapproval-rejected custom=2 slot={initial_slot}");

    // Swig's ProgramExec authority checks no signer. Veto must restrict the
    // delegated route to the agent the operator named in the policy.
    let stranger = Keypair::new();
    send(
        &rpc,
        &human,
        &[],
        vec![system_instruction::transfer(
            &human.pubkey(),
            &stranger.pubkey(),
            100_000_000,
        )],
    )?;
    send_expected_failure(
        &rpc,
        &stranger,
        gated_sign(
            gate,
            swig,
            wallet,
            stranger.pubkey(),
            policy.pubkey(),
            target,
            programdata,
            counter_key.pubkey(),
        )?,
        16,
    )?;
    let mut unsigned_agent = gated_sign(
        gate,
        swig,
        wallet,
        stranger.pubkey(),
        policy.pubkey(),
        target,
        programdata,
        counter_key.pubkey(),
    )?;
    unsigned_agent[0].accounts[6] = AccountMeta::new_readonly(agent.pubkey(), false);
    send_expected_failure(&rpc, &stranger, unsigned_agent, 15)?;
    if counter(&rpc, &counter_key.pubkey())? != 0 {
        bail!("a non-agent submitter executed the delegated target")
    }
    println!("non-agent-submitter-rejected custom=16 unsigned-agent-rejected custom=15 counter=0");

    // A freshly created policy cannot be initialized by someone who does not
    // hold its key, so a watcher cannot name themselves as the authority.
    let unclaimed_policy = Keypair::new();
    send(
        &rpc,
        &human,
        &[&unclaimed_policy],
        vec![system_instruction::create_account(
            &human.pubkey(),
            &unclaimed_policy.pubkey(),
            rent_policy,
            POLICY_LEN as u64,
            &gate,
        )],
    )?;
    let mut front_run = initialize_ix(
        gate,
        unclaimed_policy.pubkey(),
        stranger.pubkey(),
        target,
        programdata,
        swig,
        wallet,
        stranger.pubkey(),
        initial_slot,
    );
    front_run.accounts[0] = AccountMeta::new(unclaimed_policy.pubkey(), false);
    send_expected_failure(&rpc, &stranger, vec![front_run], 17)?;
    if rpc.get_account_data(&unclaimed_policy.pubkey())?[0] != 0 {
        bail!("front-run initialization changed the unclaimed policy")
    }
    println!("front-run-initialize-rejected custom=17");

    send(
        &rpc,
        &agent,
        &[],
        gated_sign(
            gate,
            swig,
            wallet,
            agent.pubkey(),
            policy.pubkey(),
            target,
            programdata,
            counter_key.pubkey(),
        )?,
    )?;
    if counter(&rpc, &counter_key.pubkey())? != 1 {
        bail!("approved delegated target did not execute exactly once")
    }
    println!("before-upgrade slot={initial_slot} counter=1");

    // Attempt the delegated target call with the SignV2 instruction alone,
    // omitting Veto's ProgramExec proof instruction entirely.
    let mut without_adapter = gated_sign(
        gate,
        swig,
        wallet,
        agent.pubkey(),
        policy.pubkey(),
        target,
        programdata,
        counter_key.pubkey(),
    )?;
    without_adapter.remove(0);
    send_expected_failure(&rpc, &agent, without_adapter, 3033)?;
    if counter(&rpc, &counter_key.pubkey())? != 1 {
        bail!("direct SignV2 without the adapter executed the target")
    }
    println!("direct-sign-without-adapter-rejected swig-custom=3033 counter=1");

    // The outer adapter must reject a transaction that reuses its proof shape
    // but asks Swig to execute another program.
    let recipient = Keypair::new();
    let proof = gate_ix(
        gate,
        swig,
        wallet,
        policy.pubkey(),
        target,
        programdata,
        agent.pubkey(),
    );
    let substituted = system_instruction::transfer(&wallet, &recipient.pubkey(), 1);
    let bypass =
        SignV2Instruction::new_program_exec(swig, wallet, agent.pubkey(), proof, substituted, 1)?;
    send_expected_failure(&rpc, &agent, bypass, 7)?;
    if counter(&rpc, &counter_key.pubkey())? != 1 {
        bail!("rejected substituted target changed the approved target counter")
    }
    println!("substitution-rejected custom=7 counter=1");

    deploy_target_upgrade(&rpc_url)?;
    let (upgraded_programdata, upgraded_slot) = programdata_and_slot(&rpc, &target)?;
    if upgraded_programdata != programdata || upgraded_slot == initial_slot {
        bail!("loader upgrade did not produce a new deployment slot")
    }
    println!("loader-upgraded old-slot={initial_slot} new-slot={upgraded_slot}");

    send_expected_failure(
        &rpc,
        &agent,
        gated_sign(
            gate,
            swig,
            wallet,
            agent.pubkey(),
            policy.pubkey(),
            target,
            upgraded_programdata,
            counter_key.pubkey(),
        )?,
        4,
    )?;
    if counter(&rpc, &counter_key.pubkey())? != 1 {
        bail!("stale deployment approval changed the target counter")
    }
    println!("stale-deployment-rejected custom=4 counter=1");

    // The approval instruction is bound to the exact slot the human reviewed.
    // If another upgrade lands after review but before submission, the
    // transaction fails instead of silently approving newer code.
    send_expected_failure(
        &rpc,
        &human,
        vec![reapprove_ix(
            gate,
            human.pubkey(),
            policy.pubkey(),
            target,
            upgraded_programdata,
            swig_program_id(),
            swig,
            wallet,
            initial_slot,
        )],
        14,
    )?;
    let policy_bytes = rpc.get_account_data(&policy.pubkey())?;
    if u64::from_le_bytes(policy_bytes[POLICY_SLOT_START..POLICY_SLOT_START + 8].try_into()?)
        != initial_slot
    {
        bail!("mismatched reviewed slot changed the pinned slot")
    }
    println!("reviewed-slot-mismatch-rejected custom=14 slot={initial_slot}");

    send(
        &rpc,
        &human,
        &[],
        vec![reapprove_ix(
            gate,
            human.pubkey(),
            policy.pubkey(),
            target,
            upgraded_programdata,
            swig_program_id(),
            swig,
            wallet,
            upgraded_slot,
        )],
    )?;
    let policy_bytes = rpc.get_account_data(&policy.pubkey())?;
    let approved_slot =
        u64::from_le_bytes(policy_bytes[POLICY_SLOT_START..POLICY_SLOT_START + 8].try_into()?);
    if approved_slot != upgraded_slot {
        bail!("human reapproval did not update the pinned slot")
    }
    send(
        &rpc,
        &agent,
        &[],
        gated_sign(
            gate,
            swig,
            wallet,
            agent.pubkey(),
            policy.pubkey(),
            target,
            upgraded_programdata,
            counter_key.pubkey(),
        )?,
    )?;
    if counter(&rpc, &counter_key.pubkey())? != 2 {
        bail!("reapproved delegated target did not resume")
    }
    println!("human-reapproved slot={upgraded_slot} counter=2");

    // Exercise the production instructions-sysvar scan against a second
    // top-level SignV2 that explicitly reuses the proof at transaction index 0.
    let mut two_top_level_calls = gated_sign(
        gate,
        swig,
        wallet,
        agent.pubkey(),
        policy.pubkey(),
        target,
        upgraded_programdata,
        counter_key.pubkey(),
    )?;
    let top_level_replay = SignV2Instruction::new_program_exec_with_ix_index(
        swig,
        wallet,
        agent.pubkey(),
        gate_ix(
            gate,
            swig,
            wallet,
            policy.pubkey(),
            target,
            upgraded_programdata,
            agent.pubkey(),
        ),
        Instruction {
            program_id: target,
            accounts: vec![AccountMeta::new(counter_key.pubkey(), false)],
            data: vec![],
        },
        1,
        0,
    )?
    .pop()
    .ok_or_else(|| anyhow!("Swig builder did not return a SignV2 instruction"))?;
    two_top_level_calls.push(top_level_replay);
    send_expected_failure(&rpc, &agent, two_top_level_calls, 10)?;
    if counter(&rpc, &counter_key.pubkey())? != 2 {
        bail!("top-level explicit-proof replay changed the target counter")
    }
    println!("top-level-proof-reuse-rejected custom=10 counter=2");

    // Next, pass the explicit-index SignV2 through an unrelated relay CPI.
    // Current Swig source independently rejects CPI entry to SignV2; verify
    // the actual runtime error rather than attributing that protection to Veto.
    let mut proof_then_direct = gated_sign(
        gate,
        swig,
        wallet,
        agent.pubkey(),
        policy.pubkey(),
        target,
        upgraded_programdata,
        counter_key.pubkey(),
    )?;
    let replay_inner = Instruction {
        program_id: target,
        accounts: vec![AccountMeta::new(counter_key.pubkey(), false)],
        data: vec![],
    };
    let replay_sign = SignV2Instruction::new_program_exec_with_ix_index(
        swig,
        wallet,
        agent.pubkey(),
        gate_ix(
            gate,
            swig,
            wallet,
            policy.pubkey(),
            target,
            upgraded_programdata,
            agent.pubkey(),
        ),
        replay_inner,
        1,
        0,
    )?
    .pop()
    .ok_or_else(|| anyhow!("Swig builder did not return a SignV2 instruction"))?;
    proof_then_direct.push(relay_swig_ix(relay, replay_sign)?);
    let nested_cpi_error = send_failure_any(&rpc, &agent, proof_then_direct)?;
    if !nested_cpi_error.contains("Custom(8)") {
        bail!("expected Swig's direct-CPI rejection Custom(8), got {nested_cpi_error}")
    }
    if counter(&rpc, &counter_key.pubkey())? != 2 {
        bail!("rejected indirect replay changed the target counter")
    }
    println!("nested-cpi-proof-reuse-rejected-by-swig custom=8 counter=2");

    // The policy must not be reusable in a second Swig wallet, even when the
    // agent installs the same gate and policy-bound ProgramExec role there.
    let create_second_wallet = CreateInstruction::new(
        second_swig,
        second_bump,
        agent.pubkey(),
        second_wallet,
        second_wallet_bump,
        AuthorityConfig {
            authority_type: AuthorityType::Ed25519,
            authority: agent.pubkey().as_ref(),
        },
        vec![ClientAction::All(All {})],
        second_id,
    )?;
    send(&rpc, &agent, &[], vec![create_second_wallet])?;
    let second_program_exec_data =
        ProgramExecAuthority::create_authority_data(&gate.to_bytes(), &role_prefix);
    let add_second_adapter = AddAuthorityInstruction::new_with_ed25519_authority(
        second_swig,
        agent.pubkey(),
        agent.pubkey(),
        0,
        AuthorityConfig {
            authority_type: AuthorityType::ProgramExec,
            authority: &second_program_exec_data,
        },
        vec![ClientAction::Program(Program {
            program_id: target.to_bytes(),
        })],
    )?;
    send(&rpc, &agent, &[], vec![add_second_adapter])?;
    send_expected_failure(
        &rpc,
        &agent,
        gated_sign(
            gate,
            second_swig,
            second_wallet,
            agent.pubkey(),
            policy.pubkey(),
            target,
            upgraded_programdata,
            counter_key.pubkey(),
        )?,
        12,
    )?;
    let cross_wallet_counter = counter(&rpc, &counter_key.pubkey())?;
    if cross_wallet_counter != 2 {
        bail!("rejected cross-wallet policy changed counter to {cross_wallet_counter}")
    }
    println!("cross-wallet-policy-reuse-rejected custom=12 counter=2");

    // Stronger replay attempt: present the approved wallet to Veto, then try
    // to consume that proof in SignV2 for the second wallet. The production
    // sysvar path must compare the proof accounts to SignV2 accounts[0..2].
    let original_wallet_proof = gate_ix(
        gate,
        swig,
        wallet,
        policy.pubkey(),
        target,
        upgraded_programdata,
        agent.pubkey(),
    );
    let target_call = Instruction {
        program_id: target,
        accounts: vec![AccountMeta::new(counter_key.pubkey(), false)],
        data: vec![],
    };
    let reused_proof = SignV2Instruction::new_program_exec(
        second_swig,
        second_wallet,
        agent.pubkey(),
        original_wallet_proof,
        target_call,
        1,
    )?;
    send_expected_failure(&rpc, &agent, reused_proof, 12)?;
    if counter(&rpc, &counter_key.pubkey())? != 2 {
        bail!("cross-wallet proof-account substitution changed the target counter")
    }
    println!("cross-wallet-proof-account-substitution-rejected custom=12 counter=2");
    println!("complete-flow-passed");
    Ok(())
}
