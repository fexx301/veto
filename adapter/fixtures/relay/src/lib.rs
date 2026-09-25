use solana_program::{
    account_info::AccountInfo,
    entrypoint,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    program::invoke,
    program_error::ProgramError,
    pubkey::Pubkey,
};

entrypoint!(process_instruction);

const RELAY_SWIG_SIGN_V2: &[u8; 8] = b"relayswg";

/// Adversarial test fixture: relays a supplied Swig SignV2 instruction through
/// CPI, to check that a proof cannot be consumed by an indirect call.
///
/// Data: `relayswg` followed by the SignV2 instruction data. Accounts 0..6 are
/// the exact accounts of that SignV2 instruction; account 6 is the Swig
/// program.
fn process_instruction(_program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let swig_data = data
        .strip_prefix(RELAY_SWIG_SIGN_V2.as_slice())
        .filter(|rest| !rest.is_empty())
        .ok_or(ProgramError::InvalidInstructionData)?;
    if accounts.len() != 7 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let metas = accounts[..6]
        .iter()
        .map(|account| {
            if account.is_writable {
                AccountMeta::new(*account.key, account.is_signer)
            } else {
                AccountMeta::new_readonly(*account.key, account.is_signer)
            }
        })
        .collect();
    invoke(
        &Instruction {
            program_id: *accounts[6].key,
            accounts: metas,
            data: swig_data.to_vec(),
        },
        accounts,
    )
}
