//! Shared client helpers for the test-token payment demo: SPL Token account
//! setup, the merchant-pay instruction, deployed-code hashing, and the
//! Veto-guarded Swig payment route. Included by the payment driver and the
//! operator server with `#[path]`.

use {
    anyhow::{anyhow, bail, Context, Result},
    solana_client::{rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig},
    solana_loader_v3_interface::{get_program_data_address, state::UpgradeableLoaderState},
    solana_sdk::{
        hash::hash,
        instruction::{AccountMeta, Instruction},
        message::Message,
        pubkey::Pubkey,
        signature::{Keypair, Signature},
        signer::Signer,
        transaction::Transaction,
    },
    solana_system_interface::instruction as system_instruction,
    std::{fs, process::Command, thread::sleep, time::Duration},
    swig_interface::{
        program_id as swig_program_id, swig_wallet_address, AddAuthorityInstruction,
        AuthorityConfig, ClientAction, CreateInstruction, SignV2Instruction,
    },
    swig_state::{
        action::{all::All, program::Program, token_limit::TokenLimit},
        authority::{programexec::ProgramExecAuthority, AuthorityType},
        swig::{swig_account_seeds, swig_wallet_address_seeds},
    },
    veto_swig_gate::{validate_next_swig_instruction, validate_swig_account_binding},
};

pub const POLICY_LEN: usize = 522;
pub const POLICY_SLOT_START: usize = 161;
pub const TOKEN_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const MINT_LEN: u64 = 82;
const TOKEN_ACCOUNT_LEN: u64 = 165;
const DECIMALS: u8 = 6;
pub const UNIT: u64 = 1_000_000;
pub const BUDGET: u64 = 500 * UNIT;
pub const PAYMENT: u64 = 10 * UNIT;

pub fn send(
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

/// Submits without preflight and requires the landed transaction to fail
/// with the given custom program error. Returns the signature and error text.
pub fn send_expected_failure(
    rpc: &RpcClient,
    payer: &Keypair,
    instructions: Vec<Instruction>,
    expected_custom: u32,
) -> Result<(Signature, String)> {
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
    for _ in 0..120 {
        if let Some(status) = rpc.get_signature_status(&signature)? {
            let error = status
                .err()
                .ok_or_else(|| anyhow!("transaction unexpectedly succeeded"))?;
            let debug = format!("{error:?}");
            if !debug.contains(&format!("Custom({expected_custom})")) {
                bail!("expected Custom({expected_custom}), got {debug}");
            }
            return Ok((signature, debug));
        }
        sleep(Duration::from_millis(250));
    }
    bail!("submitted transaction {signature} did not reach a status")
}

/// Submits without preflight and returns the landed transaction's error text;
/// fails if the transaction succeeded.
pub fn send_failure(rpc: &RpcClient, payer: &Keypair, instructions: Vec<Instruction>) -> Result<String> {
    let blockhash = rpc.get_latest_blockhash()?;
    let tx = Transaction::new(&[payer], Message::new(&instructions, Some(&payer.pubkey())), blockhash);
    let signature = rpc.send_transaction_with_config(
        &tx,
        RpcSendTransactionConfig { skip_preflight: true, ..RpcSendTransactionConfig::default() },
    )?;
    for _ in 0..120 {
        if let Some(status) = rpc.get_signature_status(&signature)? {
            return Ok(format!("{:?}", status.err().ok_or_else(|| anyhow!("transaction unexpectedly succeeded"))?));
        }
        sleep(Duration::from_millis(250));
    }
    bail!("submitted transaction {signature} did not reach a status")
}

pub fn programdata_and_slot(rpc: &RpcClient, program: &Pubkey) -> Result<(Pubkey, u64)> {
    let programdata = get_program_data_address(program);
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

/// A program is not executable in the slot it was deployed in; wait until the
/// cluster has moved past that slot before calling it.
pub fn wait_past_slot(rpc: &RpcClient, slot: u64) -> Result<()> {
    for _ in 0..240 {
        if rpc.get_slot()? > slot + 1 {
            return Ok(());
        }
        sleep(Duration::from_millis(250));
    }
    bail!("cluster did not advance past slot {slot}")
}

fn trim_trailing_zeros(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().rposition(|byte| *byte != 0).map_or(0, |i| i + 1);
    &bytes[..end]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// SHA-256 of the deployed executable, excluding loader metadata and the
/// zero padding the loader leaves after the ELF.
pub fn deployed_code_hash(rpc: &RpcClient, program: &Pubkey) -> Result<String> {
    let data = rpc.get_account_data(&get_program_data_address(program))?;
    let metadata_len = UpgradeableLoaderState::size_of_programdata_metadata();
    Ok(hex(&hash(trim_trailing_zeros(&data[metadata_len..])).to_bytes()))
}

/// Deployment slot and code hash read from one ProgramData snapshot, so an
/// approval can never pair the slot of one deployment with the hash of
/// another.
pub fn deployment_snapshot(rpc: &RpcClient, program: &Pubkey) -> Result<(u64, String)> {
    let data = rpc.get_account_data(&get_program_data_address(program))?;
    let metadata_len = UpgradeableLoaderState::size_of_programdata_metadata();
    let slot = match bincode::deserialize(&data[..metadata_len])? {
        UpgradeableLoaderState::ProgramData { slot, .. } => slot,
        _ => bail!("canonical ProgramData has an unexpected loader state"),
    };
    Ok((slot, hex(&hash(trim_trailing_zeros(&data[metadata_len..])).to_bytes())))
}

/// The same hash computed over a local build, for comparison.
pub fn file_code_hash(path: &str) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("could not read {path}"))?;
    Ok(hex(&hash(trim_trailing_zeros(&bytes)).to_bytes()))
}

pub fn create_mint_ixs(
    rpc: &RpcClient,
    payer: &Pubkey,
    mint: &Pubkey,
    authority: &Pubkey,
) -> Result<Vec<Instruction>> {
    let rent = rpc.get_minimum_balance_for_rent_exemption(MINT_LEN as usize)?;
    let mut data = vec![20, DECIMALS];
    data.extend_from_slice(authority.as_ref());
    data.push(0);
    Ok(vec![
        system_instruction::create_account(payer, mint, rent, MINT_LEN, &TOKEN_PROGRAM_ID),
        Instruction {
            program_id: TOKEN_PROGRAM_ID,
            accounts: vec![AccountMeta::new(*mint, false)],
            data,
        },
    ])
}

pub fn create_token_account_ixs(
    rpc: &RpcClient,
    payer: &Pubkey,
    account: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Result<Vec<Instruction>> {
    let rent = rpc.get_minimum_balance_for_rent_exemption(TOKEN_ACCOUNT_LEN as usize)?;
    let mut data = vec![18];
    data.extend_from_slice(owner.as_ref());
    Ok(vec![
        system_instruction::create_account(payer, account, rent, TOKEN_ACCOUNT_LEN, &TOKEN_PROGRAM_ID),
        Instruction {
            program_id: TOKEN_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(*account, false),
                AccountMeta::new_readonly(*mint, false),
            ],
            data,
        },
    ])
}

pub fn mint_to_ix(mint: &Pubkey, destination: &Pubkey, authority: &Pubkey, amount: u64) -> Instruction {
    let mut data = vec![7];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*mint, false),
            AccountMeta::new(*destination, false),
            AccountMeta::new_readonly(*authority, true),
        ],
        data,
    }
}

pub fn balance(rpc: &RpcClient, token_account: &Pubkey) -> Result<u64> {
    let data = rpc.get_account_data(token_account)?;
    Ok(u64::from_le_bytes(data[64..72].try_into()?))
}

pub fn units(amount: u64) -> String {
    format!("{}.{:02}", amount / UNIT, (amount % UNIT) / (UNIT / 100))
}

pub fn pay_ix(pay: Pubkey, source: Pubkey, merchant: Pubkey, wallet: Pubkey, amount: u64) -> Instruction {
    Instruction {
        program_id: pay,
        accounts: vec![
            AccountMeta::new(source, false),
            AccountMeta::new(merchant, false),
            AccountMeta::new_readonly(wallet, true),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: amount.to_le_bytes().to_vec(),
    }
}

/// The agent's permissions on either wallet: call merchant-pay, spending at
/// most the test-token budget.
pub fn agent_actions(pay: &Pubkey, mint: &Pubkey) -> Vec<ClientAction> {
    vec![
        ClientAction::Program(Program {
            program_id: pay.to_bytes(),
        }),
        ClientAction::TokenLimit(TokenLimit {
            token_mint: mint.to_bytes(),
            current_amount: BUDGET,
        }),
    ]
}

/// Swig config/wallet addresses for a fixed demo id, plus the create
/// instruction with `root` as the `All`-permission root authority.
pub fn swig_create(id: [u8; 32], payer: Pubkey, root: Pubkey) -> Result<(Pubkey, Pubkey, Instruction)> {
    let (swig, bump) = Pubkey::find_program_address(&swig_account_seeds(&id), &swig_program_id());
    let wallet = swig_wallet_address(&swig);
    let (derived, wallet_bump) =
        Pubkey::find_program_address(&swig_wallet_address_seeds(swig.as_ref()), &swig_program_id());
    if derived != wallet {
        bail!("Swig interface and state wallet seed helpers disagree")
    }
    let create = CreateInstruction::new(
        swig,
        bump,
        payer,
        wallet,
        wallet_bump,
        AuthorityConfig {
            authority_type: AuthorityType::Ed25519,
            authority: root.as_ref(),
        },
        vec![ClientAction::All(All {})],
        id,
    )?;
    Ok((swig, wallet, create))
}

/// Upgrades `program` with the given build, signed by its upgrade authority.
pub fn deploy(solana_bin: &str, rpc_url: &str, authority_path: &str, program_keypair: &str, so: &str) -> Result<String> {
    let output = Command::new(solana_bin)
        .args([
            "program",
            "deploy",
            "--url",
            rpc_url,
            "--keypair",
            authority_path,
            "--upgrade-authority",
            authority_path,
            "--program-id",
            program_keypair,
            so,
        ])
        .output()
        .context("could not run the Solana loader upgrade")?;
    if !output.status.success() {
        bail!("upgrade failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn approval_data(discriminator: u8, reviewed_slot: u64) -> Vec<u8> {
    let mut data = vec![discriminator];
    data.extend_from_slice(&reviewed_slot.to_le_bytes());
    data
}

/// The Veto-guarded Swig wallet and its policy.
pub struct Guarded {
    pub gate: Pubkey,
    pub swig: Pubkey,
    pub wallet: Pubkey,
    pub policy: Pubkey,
    pub pay: Pubkey,
}

impl Guarded {
    fn gate_ix(&self, programdata: Pubkey, agent: Pubkey) -> Instruction {
        let mut data = vec![2];
        data.extend_from_slice(self.policy.as_ref());
        Instruction {
            program_id: self.gate,
            accounts: vec![
                AccountMeta::new_readonly(self.swig, false),
                AccountMeta::new_readonly(self.wallet, false),
                AccountMeta::new_readonly(self.policy, false),
                AccountMeta::new_readonly(self.pay, false),
                AccountMeta::new_readonly(programdata, false),
                AccountMeta::new_readonly(solana_sdk::sysvar::instructions::id(), false),
                AccountMeta::new_readonly(agent, true),
            ],
            data,
        }
    }

    /// Veto proof followed by Swig SignV2 for one inner instruction.
    pub fn payment(&self, agent: Pubkey, inner: Instruction) -> Result<Vec<Instruction>> {
        self.payment_via(agent, inner, &[])
    }

    /// As `payment`, also passing Veto the ProgramData of downstream programs
    /// the inner instruction can reach, so it can check their deployments.
    pub fn payment_via(&self, agent: Pubkey, inner: Instruction, route: &[Pubkey]) -> Result<Vec<Instruction>> {
        let programdata = get_program_data_address(&self.pay);
        let mut proof = self.gate_ix(programdata, agent);
        for meta in &inner.accounts {
            proof.accounts.push(AccountMeta::new_readonly(meta.pubkey, false));
        }
        // SPL Token is always in a payment route; Veto checks its deployment.
        for program in std::iter::once(&TOKEN_PROGRAM_ID).chain(route) {
            proof.accounts.push(AccountMeta::new_readonly(get_program_data_address(program), false));
        }
        let instructions = SignV2Instruction::new_program_exec(self.swig, self.wallet, agent, proof, inner, 1)?;
        validate_next_swig_instruction(&instructions[1], &swig_program_id(), &self.pay)
            .map_err(|error| anyhow!("Veto parser rejected generated payload: {error:?}"))?;
        validate_swig_account_binding(&instructions[1], &self.swig, &self.wallet)
            .map_err(|error| anyhow!("Veto wallet binding rejected generated payload: {error:?}"))?;
        Ok(instructions)
    }

    /// `initialize` (discriminator 0, names the agent) or `reapprove` (1).
    pub fn approval_ix(&self, discriminator: u8, operator: Pubkey, agent: Pubkey, reviewed_slot: u64) -> Instruction {
        let mut accounts = vec![
            AccountMeta::new(self.policy, discriminator == 0),
            AccountMeta::new_readonly(operator, true),
            AccountMeta::new_readonly(self.pay, false),
            AccountMeta::new_readonly(get_program_data_address(&self.pay), false),
            AccountMeta::new_readonly(swig_program_id(), false),
            AccountMeta::new_readonly(self.swig, false),
            AccountMeta::new_readonly(self.wallet, false),
        ];
        if discriminator == 0 {
            accounts.push(AccountMeta::new_readonly(agent, false));
        }
        Instruction {
            program_id: self.gate,
            accounts,
            data: approval_data(discriminator, reviewed_slot),
        }
    }

    /// Approves (or re-approves) a downstream program at its current slot.
    pub fn approve_program_ix(&self, operator: Pubkey, program: Pubkey, reviewed_slot: u64) -> Instruction {
        Instruction {
            program_id: self.gate,
            accounts: vec![
                AccountMeta::new(self.policy, false),
                AccountMeta::new_readonly(operator, true),
                AccountMeta::new_readonly(program, false),
                AccountMeta::new_readonly(get_program_data_address(&program), false),
                AccountMeta::new_readonly(swig_program_id(), false),
                AccountMeta::new_readonly(self.swig, false),
                AccountMeta::new_readonly(self.wallet, false),
            ],
            data: approval_data(3, reviewed_slot),
        }
    }

    /// Creates the policy account, initializes it, and adds the Veto-bound
    /// Swig role in one transaction. The operator signs as Swig root and as
    /// the policy authority; the policy key signs its own initialization.
    pub fn setup_ixs(
        &self,
        rpc: &RpcClient,
        payer: Pubkey,
        operator: Pubkey,
        agent: Pubkey,
        mint: Pubkey,
        reviewed_slot: u64,
    ) -> Result<Vec<Instruction>> {
        let rent = rpc.get_minimum_balance_for_rent_exemption(POLICY_LEN)?;
        // SPL Token runs on the upgradeable loader, so the payment route needs
        // its current deployment approved like any other downstream program.
        let (_, token_slot) = programdata_and_slot(rpc, &TOKEN_PROGRAM_ID)?;
        let mut role_prefix = vec![2];
        role_prefix.extend_from_slice(self.policy.as_ref());
        Ok(vec![
            system_instruction::create_account(&payer, &self.policy, rent, POLICY_LEN as u64, &self.gate),
            self.approval_ix(0, operator, agent, reviewed_slot),
            AddAuthorityInstruction::new_with_ed25519_authority(
                self.swig,
                payer,
                operator,
                0,
                AuthorityConfig {
                    authority_type: AuthorityType::ProgramExec,
                    authority: &ProgramExecAuthority::create_authority_data(
                        &self.gate.to_bytes(),
                        &role_prefix,
                    ),
                },
                agent_actions(&self.pay, &mint),
            )?,
            self.approve_program_ix(operator, TOKEN_PROGRAM_ID, token_slot),
        ])
    }

    /// The approved deployment slot, or `None` before initialization.
    pub fn approved_slot(&self, rpc: &RpcClient) -> Result<Option<u64>> {
        let Some(account) = rpc
            .get_account_with_commitment(&self.policy, rpc.commitment())?
            .value
        else {
            return Ok(None);
        };
        if account.data.first() != Some(&1) {
            return Ok(None);
        }
        Ok(Some(u64::from_le_bytes(
            account.data[POLICY_SLOT_START..POLICY_SLOT_START + 8].try_into()?,
        )))
    }
}

/// Swig SignV2 for the plain-allowlist wallet: the agent's own key authorizes.
pub fn plain_payment(swig: Pubkey, wallet: Pubkey, agent: Pubkey, inner: Instruction) -> Result<Instruction> {
    SignV2Instruction::new_ed25519(swig, wallet, agent, inner, 1)
}

/// Adds the plain-allowlist agent role to a wallet whose root is `root`.
pub fn add_plain_agent_ix(swig: Pubkey, payer: Pubkey, root: Pubkey, agent: Pubkey, pay: Pubkey, mint: Pubkey) -> Result<Instruction> {
    AddAuthorityInstruction::new_with_ed25519_authority(
        swig,
        payer,
        root,
        0,
        AuthorityConfig {
            authority_type: AuthorityType::Ed25519,
            authority: agent.as_ref(),
        },
        agent_actions(&pay, &mint),
    )
}
