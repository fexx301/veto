use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, instruction::Instruction,
    msg, program::invoke, program_error::ProgramError, pubkey::Pubkey,
};

entrypoint!(process_instruction);

/// Test fixture for the runtime rule route pinning relies on. Data `[0]` is a
/// no-op (used to place another program's ID in the transaction). Data
/// `[1, program_id(32)]` tries to invoke that program *without* it being one of
/// this instruction's accounts; the runtime must reject the call.
fn process_instruction(_program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    match data {
        [0, ..] => Ok(()),
        [1, rest @ ..] if rest.len() == 32 => {
            let callee = Pubkey::new_from_array(rest.try_into().unwrap());
            msg!("sneaky-cpi invoking {} with {} accounts passed", callee, accounts.len());
            invoke(&Instruction { program_id: callee, accounts: vec![], data: 10u64.to_le_bytes().to_vec() }, accounts)
        },
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
