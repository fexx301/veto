use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    msg,
    program::invoke,
    pubkey::Pubkey,
};

entrypoint!(process_instruction);

/// Demo first-hop program: forwards `pay(amount)` to a downstream payment
/// program. The agent's wallet approves this router; the downstream program is
/// the one that gets upgraded in the route-pinning demo, while this router's
/// code and deployment slot never change.
///
/// Accounts: payer token account, merchant token account, payer authority
/// (signer), SPL Token program, downstream payment program.
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
    let payment_program = next_account_info(&mut iter)?;
    msg!("payment-router forwarding to {}", payment_program.key);
    invoke(
        &Instruction {
            program_id: *payment_program.key,
            accounts: vec![
                AccountMeta::new(*source.key, false),
                AccountMeta::new(*merchant.key, false),
                AccountMeta::new_readonly(*authority.key, true),
                AccountMeta::new_readonly(*token_program.key, false),
            ],
            data: instruction_data.to_vec(),
        },
        &[
            source.clone(),
            merchant.clone(),
            authority.clone(),
            token_program.clone(),
            payment_program.clone(),
        ],
    )
}
