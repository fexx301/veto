//! Veto adapter for Swig `ProgramExec` authorities.
//!
//! Scope: it binds one first-hop compact instruction in the immediately following
//! Swig SignV2 call to one upgradeable target program and its ProgramData slot.
//! It deliberately does not claim to constrain CPIs made by that target.

use {
    bincode::deserialize,
    solana_loader_v3_interface::{get_program_data_address, state::UpgradeableLoaderState},
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        entrypoint,
        entrypoint::ProgramResult,
        instruction::Instruction,
        msg,
        program_error::ProgramError,
        pubkey::Pubkey,
        sysvar::instructions::{load_current_index_checked, load_instruction_at_checked},
    },
    solana_sdk_ids::bpf_loader_upgradeable,
};

entrypoint!(process_instruction);

/// Header (201 bytes), route count (1 byte), then up to `MAX_ROUTE` route
/// entries of (program, approved deployment slot).
pub const POLICY_LEN: usize = 202 + MAX_ROUTE * ROUTE_ENTRY_LEN;
/// Offset of the approved deployment slot (little-endian u64) in the policy.
pub const POLICY_SLOT_START: usize = 161;
const MAX_ROUTE: usize = 8;
const ROUTE_ENTRY_LEN: usize = 40;
const ROUTE_COUNT: usize = 201;
const INITIALIZED: u8 = 1;
const SIGN_V2: u16 = 11;

mod error {
    pub const INVALID_POLICY: u32 = 1;
    pub const UNAUTHORIZED_REAPPROVAL: u32 = 2;
    pub const TARGET_MISMATCH: u32 = 3;
    pub const DEPLOYMENT_CHANGED: u32 = 4;
    pub const EXPECTED_NEXT_SWIG: u32 = 5;
    pub const INVALID_SWIG_PAYLOAD: u32 = 6;
    pub const COMPACT_TARGET_MISMATCH: u32 = 7;
    pub const MULTI_INSTRUCTION: u32 = 8;
    pub const SWIG_MISMATCH: u32 = 9;
    pub const LATER_SWIG_INSTRUCTION: u32 = 10;
    pub const POLICY_ROLE_MISMATCH: u32 = 11;
    pub const SWIG_CONFIG_MISMATCH: u32 = 12;
    pub const SWIG_WALLET_MISMATCH: u32 = 13;
    pub const REVIEWED_SLOT_MISMATCH: u32 = 14;
    pub const AGENT_NOT_SIGNER: u32 = 15;
    pub const AGENT_MISMATCH: u32 = 16;
    pub const POLICY_NOT_SIGNER: u32 = 17;
    pub const ROUTE_ACCOUNT_MISSING: u32 = 18;
    pub const ROUTE_PROGRAM_NOT_APPROVED: u32 = 19;
    pub const ROUTE_PROGRAM_CHANGED: u32 = 20;
    pub const ROUTE_FULL: u32 = 21;
    pub const UNSUPPORTED_LOADER: u32 = 22;
}

fn gate_error(code: u32) -> ProgramError {
    ProgramError::Custom(code)
}

/// Policy bytes: initialized, human authority, target program, Swig program,
/// the approved Swig config and wallet PDA, the target's deployment slot, and
/// the one agent key allowed to submit the delegated action.
struct Policy<'a> {
    bytes: &'a [u8],
}

impl<'a> Policy<'a> {
    fn load(bytes: &'a [u8]) -> Result<Self, ProgramError> {
        if bytes.len() != POLICY_LEN || bytes[0] != INITIALIZED {
            return Err(gate_error(error::INVALID_POLICY));
        }
        Ok(Self { bytes })
    }

    fn authority(&self) -> [u8; 32] {
        self.bytes[1..33].try_into().unwrap()
    }

    fn target(&self) -> [u8; 32] {
        self.bytes[33..65].try_into().unwrap()
    }

    fn swig(&self) -> [u8; 32] {
        self.bytes[65..97].try_into().unwrap()
    }

    fn swig_config(&self) -> [u8; 32] {
        self.bytes[97..129].try_into().unwrap()
    }

    fn swig_wallet(&self) -> [u8; 32] {
        self.bytes[129..161].try_into().unwrap()
    }

    fn slot(&self) -> u64 {
        u64::from_le_bytes(self.bytes[POLICY_SLOT_START..POLICY_SLOT_START + 8].try_into().unwrap())
    }

    fn agent(&self) -> [u8; 32] {
        self.bytes[169..201].try_into().unwrap()
    }

    /// Approved slot of a downstream program the delegated call may reach.
    fn route_slot(&self, program: &Pubkey) -> Option<u64> {
        route_entries(self.bytes).find_map(|(key, slot)| (key == program.to_bytes()).then_some(slot))
    }
}

fn route_entries(bytes: &[u8]) -> impl Iterator<Item = ([u8; 32], u64)> + '_ {
    let count = (bytes[ROUTE_COUNT] as usize).min(MAX_ROUTE);
    (0..count).map(move |index| {
        let start = ROUTE_COUNT + 1 + index * ROUTE_ENTRY_LEN;
        (
            bytes[start..start + 32].try_into().unwrap(),
            u64::from_le_bytes(bytes[start + 32..start + 40].try_into().unwrap()),
        )
    })
}

fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    match data {
        [0, expected_slot @ ..] => initialize(program_id, accounts, reviewed_slot(expected_slot)?),
        [1, expected_slot @ ..] => reapprove(program_id, accounts, reviewed_slot(expected_slot)?),
        [2, policy_binding @ ..] => authorize_next_swig(program_id, accounts, policy_binding),
        [3, expected_slot @ ..] => {
            approve_route_program(program_id, accounts, reviewed_slot(expected_slot)?)
        },
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn reviewed_slot(bytes: &[u8]) -> Result<u64, ProgramError> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    Ok(u64::from_le_bytes(bytes))
}

/// Accounts: writable policy signer, human signer, target program,
/// ProgramData, Swig program, Swig config, Swig wallet PDA, agent.
///
/// The policy account must sign so that only the party that created it can
/// initialize it. Otherwise anyone watching the chain could initialize a
/// freshly created policy first and name themselves as its authority.
fn initialize(program_id: &Pubkey, accounts: &[AccountInfo], expected_slot: u64) -> ProgramResult {
    let mut iter = accounts.iter();
    let policy = next_account_info(&mut iter)?;
    let human = next_account_info(&mut iter)?;
    let target = next_account_info(&mut iter)?;
    let programdata = next_account_info(&mut iter)?;
    let swig = next_account_info(&mut iter)?;
    let swig_config = next_account_info(&mut iter)?;
    let swig_wallet = next_account_info(&mut iter)?;
    let agent = next_account_info(&mut iter)?;
    if policy.owner != program_id || !policy.is_writable || !human.is_signer {
        return Err(gate_error(error::INVALID_POLICY));
    }
    if !policy.is_signer {
        return Err(gate_error(error::POLICY_NOT_SIGNER));
    }
    let slot = deployment_slot(target, programdata)?;
    if slot != expected_slot {
        return Err(gate_error(error::REVIEWED_SLOT_MISMATCH));
    }
    let mut bytes = policy.try_borrow_mut_data()?;
    if bytes.len() != POLICY_LEN || bytes[0] != 0 {
        return Err(gate_error(error::INVALID_POLICY));
    }
    bytes[0] = INITIALIZED;
    bytes[1..33].copy_from_slice(human.key.as_ref());
    bytes[33..65].copy_from_slice(target.key.as_ref());
    bytes[65..97].copy_from_slice(swig.key.as_ref());
    bytes[97..129].copy_from_slice(swig_config.key.as_ref());
    bytes[129..161].copy_from_slice(swig_wallet.key.as_ref());
    bytes[161..169].copy_from_slice(&slot.to_le_bytes());
    bytes[169..201].copy_from_slice(agent.key.as_ref());
    Ok(())
}

/// Accounts: writable policy, human signer, target program, ProgramData, Swig
/// program, Swig config, Swig wallet PDA.
fn reapprove(program_id: &Pubkey, accounts: &[AccountInfo], expected_slot: u64) -> ProgramResult {
    let mut iter = accounts.iter();
    let policy_account = next_account_info(&mut iter)?;
    let human = next_account_info(&mut iter)?;
    let target = next_account_info(&mut iter)?;
    let programdata = next_account_info(&mut iter)?;
    let swig = next_account_info(&mut iter)?;
    let swig_config = next_account_info(&mut iter)?;
    let swig_wallet = next_account_info(&mut iter)?;
    if policy_account.owner != program_id || !policy_account.is_writable || !human.is_signer {
        return Err(gate_error(error::INVALID_POLICY));
    }
    let mut bytes = policy_account.try_borrow_mut_data()?;
    let policy = Policy::load(&bytes)?;
    if policy.authority() != human.key.to_bytes() {
        return Err(gate_error(error::UNAUTHORIZED_REAPPROVAL));
    }
    if policy.target() != target.key.to_bytes() {
        return Err(gate_error(error::TARGET_MISMATCH));
    }
    if policy.swig() != swig.key.to_bytes() {
        return Err(gate_error(error::SWIG_MISMATCH));
    }
    if policy.swig_config() != swig_config.key.to_bytes() {
        return Err(gate_error(error::SWIG_CONFIG_MISMATCH));
    }
    if policy.swig_wallet() != swig_wallet.key.to_bytes() {
        return Err(gate_error(error::SWIG_WALLET_MISMATCH));
    }
    let slot = deployment_slot(target, programdata)?;
    if slot != expected_slot {
        return Err(gate_error(error::REVIEWED_SLOT_MISMATCH));
    }
    bytes[161..169].copy_from_slice(&slot.to_le_bytes());
    Ok(())
}

/// Accounts: writable policy, human signer, program, its ProgramData, Swig
/// program, Swig config, Swig wallet PDA.
///
/// Approves (or re-approves after an upgrade) a program the delegated call may
/// reach downstream of the target, pinned to its current deployment slot.
fn approve_route_program(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    expected_slot: u64,
) -> ProgramResult {
    let mut iter = accounts.iter();
    let policy_account = next_account_info(&mut iter)?;
    let human = next_account_info(&mut iter)?;
    let program = next_account_info(&mut iter)?;
    let programdata = next_account_info(&mut iter)?;
    let swig = next_account_info(&mut iter)?;
    let swig_config = next_account_info(&mut iter)?;
    let swig_wallet = next_account_info(&mut iter)?;
    if policy_account.owner != program_id || !policy_account.is_writable || !human.is_signer {
        return Err(gate_error(error::INVALID_POLICY));
    }
    let mut bytes = policy_account.try_borrow_mut_data()?;
    let policy = Policy::load(&bytes)?;
    if policy.authority() != human.key.to_bytes() {
        return Err(gate_error(error::UNAUTHORIZED_REAPPROVAL));
    }
    if policy.swig() != swig.key.to_bytes() {
        return Err(gate_error(error::SWIG_MISMATCH));
    }
    if policy.swig_config() != swig_config.key.to_bytes() {
        return Err(gate_error(error::SWIG_CONFIG_MISMATCH));
    }
    if policy.swig_wallet() != swig_wallet.key.to_bytes() {
        return Err(gate_error(error::SWIG_WALLET_MISMATCH));
    }
    let slot = deployment_slot(program, programdata)?;
    if slot != expected_slot {
        return Err(gate_error(error::REVIEWED_SLOT_MISMATCH));
    }
    if policy.target() == program.key.to_bytes() {
        bytes[161..169].copy_from_slice(&slot.to_le_bytes());
        return Ok(());
    }
    let existing = route_entries(&bytes).position(|(key, _)| key == program.key.to_bytes());
    let index = match existing {
        Some(index) => index,
        None => {
            let count = bytes[ROUTE_COUNT] as usize;
            if count >= MAX_ROUTE {
                return Err(gate_error(error::ROUTE_FULL));
            }
            bytes[ROUTE_COUNT] = count as u8 + 1;
            count
        },
    };
    let start = ROUTE_COUNT + 1 + index * ROUTE_ENTRY_LEN;
    bytes[start..start + 32].copy_from_slice(program.key.as_ref());
    bytes[start + 32..start + 40].copy_from_slice(&slot.to_le_bytes());
    Ok(())
}

/// Accounts: Swig config, Swig wallet PDA, policy, target program, ProgramData,
/// instructions sysvar, agent signer, then every account of the delegated inner
/// instruction plus the ProgramData of each approved downstream program. The
/// first two positions are intentional:
/// Swig's ProgramExec authority authenticates them against its own SignV2
/// accounts. Swig's ProgramExec checks no signer, so the agent signature here
/// is what restricts the delegated route to the agent the operator named.
///
/// The caller configures the Swig ProgramExec role with the exact 33-byte
/// prefix `[2, policy_pubkey]`. That binds this policy to that delegated role;
/// a different policy cannot be substituted by an agent.
fn authorize_next_swig(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    policy_binding: &[u8],
) -> ProgramResult {
    let mut iter = accounts.iter();
    let swig_config = next_account_info(&mut iter)?;
    let swig_wallet = next_account_info(&mut iter)?;
    let policy_account = next_account_info(&mut iter)?;
    let target = next_account_info(&mut iter)?;
    let programdata = next_account_info(&mut iter)?;
    let instructions = next_account_info(&mut iter)?;
    let agent = next_account_info(&mut iter)?;
    if policy_account.owner != program_id {
        return Err(gate_error(error::INVALID_POLICY));
    }
    if policy_binding != policy_account.key.as_ref() {
        return Err(gate_error(error::POLICY_ROLE_MISMATCH));
    }
    let policy_bytes = policy_account.try_borrow_data()?;
    let policy = Policy::load(&policy_bytes)?;
    if policy.swig_config() != swig_config.key.to_bytes() {
        return Err(gate_error(error::SWIG_CONFIG_MISMATCH));
    }
    if policy.swig_wallet() != swig_wallet.key.to_bytes() {
        return Err(gate_error(error::SWIG_WALLET_MISMATCH));
    }
    if !agent.is_signer {
        return Err(gate_error(error::AGENT_NOT_SIGNER));
    }
    if policy.agent() != agent.key.to_bytes() {
        return Err(gate_error(error::AGENT_MISMATCH));
    }
    if policy.target() != target.key.to_bytes() {
        return Err(gate_error(error::TARGET_MISMATCH));
    }
    if deployment_slot(target, programdata)? != policy.slot() {
        return Err(gate_error(error::DEPLOYMENT_CHANGED));
    }
    let current = load_current_index_checked(instructions)? as usize;
    let next = load_instruction_at_checked(current + 1, instructions)
        .map_err(|_| gate_error(error::EXPECTED_NEXT_SWIG))?;
    let expected_swig = Pubkey::new_from_array(policy.swig());
    validate_next_swig_instruction(&next, &expected_swig, target.key)?;
    validate_swig_account_binding(&next, swig_config.key, swig_wallet.key)?;
    check_route(accounts, &policy, target.key, &inner_account_keys(&next)?)?;
    reject_later_swig_instructions(instructions, current + 2, &expected_swig)
}

/// Confirms the following SignV2 executes from the same config and wallet
/// whose policy was checked by this proof. Otherwise an agent could present
/// the approved wallet to the gate, then consume the proof from another
/// Swig wallet whose ProgramExec role references the same policy.
pub fn validate_swig_account_binding(
    next: &Instruction,
    expected_config: &Pubkey,
    expected_wallet: &Pubkey,
) -> ProgramResult {
    if next.accounts.first().map(|meta| meta.pubkey) != Some(*expected_config) {
        return Err(gate_error(error::SWIG_CONFIG_MISMATCH));
    }
    if next.accounts.get(1).map(|meta| meta.pubkey) != Some(*expected_wallet) {
        return Err(gate_error(error::SWIG_WALLET_MISMATCH));
    }
    Ok(())
}

/// Validates the exact compact payload Swig will execute. Public for host tests
/// that use Swig's real instruction builder; runtime calls it above.
pub fn validate_next_swig_instruction(
    next: &Instruction,
    expected_swig: &Pubkey,
    expected_target: &Pubkey,
) -> ProgramResult {
    if next.program_id != *expected_swig {
        return Err(gate_error(error::EXPECTED_NEXT_SWIG));
    }
    if next.data.len() < 9 || u16::from_le_bytes([next.data[0], next.data[1]]) != SIGN_V2 {
        return Err(gate_error(error::INVALID_SWIG_PAYLOAD));
    }
    let payload_len = u16::from_le_bytes([next.data[2], next.data[3]]) as usize;
    let payload_end = 8usize
        .checked_add(payload_len)
        .ok_or_else(|| gate_error(error::INVALID_SWIG_PAYLOAD))?;
    if payload_end >= next.data.len() {
        return Err(gate_error(error::INVALID_SWIG_PAYLOAD));
    }
    let payload = &next.data[8..payload_end];
    // ProgramExec accepts a one- or two-byte authority payload only.
    if payload.is_empty() {
        return Err(gate_error(error::INVALID_SWIG_PAYLOAD));
    }
    // The one-byte form authenticates only the immediately preceding proof.
    // Swig's optional second byte permits a later SignV2 to point back to an
    // older proof, so it cannot be used with this one-action adapter.
    if next.data.len() - payload_end != 1 || payload[0] != 1 {
        return Err(gate_error(error::MULTI_INSTRUCTION));
    }
    if payload.len() < 3 {
        return Err(gate_error(error::INVALID_SWIG_PAYLOAD));
    }
    let program_index = payload[1] as usize;
    let account_count = payload[2] as usize;
    let accounts_end = 3usize
        .checked_add(account_count)
        .ok_or_else(|| gate_error(error::INVALID_SWIG_PAYLOAD))?;
    if accounts_end + 2 > payload.len() {
        return Err(gate_error(error::INVALID_SWIG_PAYLOAD));
    }
    let data_len = u16::from_le_bytes([payload[accounts_end], payload[accounts_end + 1]]) as usize;
    if accounts_end + 2 + data_len != payload.len() {
        return Err(gate_error(error::INVALID_SWIG_PAYLOAD));
    }
    let target = next
        .accounts
        .get(program_index)
        .ok_or_else(|| gate_error(error::INVALID_SWIG_PAYLOAD))?;
    if target.pubkey != *expected_target {
        msg!("veto-swig: inner target does not match policy");
        return Err(gate_error(error::COMPACT_TARGET_MISMATCH));
    }
    Ok(())
}

/// Rejects another Swig instruction after the protected SignV2 call. Without
/// this scan, that later call could use Swig's explicit-index payload to reuse
/// this proof without invoking the gate again. This deliberately limits a
/// Veto-protected transaction to one Swig instruction.
fn reject_later_swig_instructions(
    instructions: &AccountInfo,
    start: usize,
    expected_swig: &Pubkey,
) -> ProgramResult {
    for index in start..=u8::MAX as usize {
        match load_instruction_at_checked(index, instructions) {
            Ok(instruction) if instruction.program_id == *expected_swig => {
                return Err(gate_error(error::LATER_SWIG_INSTRUCTION));
            },
            Ok(_) => {},
            Err(_) => break,
        }
    }
    Ok(())
}

/// Host-testable form of the transaction scan above. Runtime reads the real
/// instructions sysvar; this checks instructions produced by Swig's builder.
pub fn reject_later_swig_instruction_list(
    later: &[Instruction],
    expected_swig: &Pubkey,
) -> ProgramResult {
    if later
        .iter()
        .any(|instruction| instruction.program_id == *expected_swig)
    {
        return Err(gate_error(error::LATER_SWIG_INSTRUCTION));
    }
    Ok(())
}

/// Every account the delegated inner instruction receives. A program can only
/// invoke programs among its instruction accounts (the runtime rejects an
/// unknown callee), so this set contains every program the call can reach.
pub fn inner_account_keys(next: &Instruction) -> Result<Vec<Pubkey>, ProgramError> {
    let payload_len = u16::from_le_bytes([next.data[2], next.data[3]]) as usize;
    let payload = next
        .data
        .get(8..8 + payload_len)
        .ok_or_else(|| gate_error(error::INVALID_SWIG_PAYLOAD))?;
    let account_count = *payload.get(2).ok_or_else(|| gate_error(error::INVALID_SWIG_PAYLOAD))? as usize;
    let indices = payload
        .get(3..3 + account_count)
        .ok_or_else(|| gate_error(error::INVALID_SWIG_PAYLOAD))?;
    indices
        .iter()
        .map(|index| {
            next.accounts
                .get(*index as usize)
                .map(|meta| meta.pubkey)
                .ok_or_else(|| gate_error(error::INVALID_SWIG_PAYLOAD))
        })
        .collect()
}

/// Requires every program the delegated call can reach to be approved.
///
/// Programs are recognized by their owning loader rather than the executable
/// flag. Upgradeable-loader programs must be approved at their current
/// deployment slot, whether or not they still have an upgrade authority: a
/// finalized program is not trusted merely because nobody can upgrade it, since
/// anyone can deploy and finalize a new program and pass it into a route.
/// Native builtins cannot be user-deployed and are allowed. Legacy-loader and
/// loader-v4 programs are refused. Each inner account must be passed to this
/// instruction so its owner and state can be read.
fn check_route(
    accounts: &[AccountInfo],
    policy: &Policy,
    target: &Pubkey,
    inner_keys: &[Pubkey],
) -> ProgramResult {
    let find = |key: &Pubkey| accounts.iter().find(|account| account.key == key);
    for key in inner_keys {
        if key == target {
            continue;
        }
        let account = find(key).ok_or_else(|| gate_error(error::ROUTE_ACCOUNT_MISSING))?;
        let owner = account.owner;
        if *owner == bpf_loader_upgradeable::id() {
            // Fail closed on loader-owned bytes that do not decode.
            let state: UpgradeableLoaderState = deserialize(&account.try_borrow_data()?)
                .map_err(|_| gate_error(error::UNSUPPORTED_LOADER))?;
            if !matches!(state, UpgradeableLoaderState::Program { .. }) {
                continue; // ProgramData and buffers are not callable
            }
            let approved = policy
                .route_slot(key)
                .ok_or_else(|| gate_error(error::ROUTE_PROGRAM_NOT_APPROVED))?;
            let programdata = find(&get_program_data_address(key))
                .ok_or_else(|| gate_error(error::ROUTE_ACCOUNT_MISSING))?;
            if deployment_slot(account, programdata)? != approved {
                msg!("veto-swig: downstream program {} changed", key);
                return Err(gate_error(error::ROUTE_PROGRAM_CHANGED));
            }
        } else if *owner == solana_sdk_ids::bpf_loader::id()
            || *owner == solana_sdk_ids::bpf_loader_deprecated::id()
            || *owner == solana_sdk_ids::loader_v4::id()
        {
            return Err(gate_error(error::UNSUPPORTED_LOADER));
        }
        // Native builtins and ordinary data accounts need no approval.
    }
    Ok(())
}

fn deployment_slot(target: &AccountInfo, programdata: &AccountInfo) -> Result<u64, ProgramError> {
    if !target.executable
        || target.owner != &bpf_loader_upgradeable::id()
        || programdata.owner != &bpf_loader_upgradeable::id()
        || *programdata.key != get_program_data_address(target.key)
    {
        return Err(gate_error(error::TARGET_MISMATCH));
    }
    let target_state: UpgradeableLoaderState =
        deserialize(&target.try_borrow_data()?).map_err(|_| gate_error(error::INVALID_POLICY))?;
    if !matches!(target_state, UpgradeableLoaderState::Program { programdata_address } if programdata_address == *programdata.key)
    {
        return Err(gate_error(error::TARGET_MISMATCH));
    }
    let metadata_len = UpgradeableLoaderState::size_of_programdata_metadata();
    let data = programdata.try_borrow_data()?;
    let metadata: UpgradeableLoaderState = deserialize(
        data.get(..metadata_len)
            .ok_or_else(|| gate_error(error::INVALID_POLICY))?,
    )
    .map_err(|_| gate_error(error::INVALID_POLICY))?;
    match metadata {
        UpgradeableLoaderState::ProgramData { slot, .. } => Ok(slot),
        _ => Err(gate_error(error::TARGET_MISMATCH)),
    }
}
