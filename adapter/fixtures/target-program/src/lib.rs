use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
};

entrypoint!(process_instruction);

/// Valueless demo action: increment a target-owned counter.
///
/// The visible state change proves whether the delegated CPI ran. The program
/// intentionally has no authority, token, or transfer behavior.
fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    _instruction_data: &[u8],
) -> ProgramResult {
    let counter = next_account_info(&mut accounts.iter())?;
    if counter.owner != program_id {
        return Err(ProgramError::IncorrectProgramId);
    }
    if !counter.is_writable {
        return Err(ProgramError::InvalidArgument);
    }

    let mut data = counter.try_borrow_mut_data()?;
    let current = u64::from_le_bytes(
        data.get(..8)
            .ok_or(ProgramError::AccountDataTooSmall)?
            .try_into()
            .map_err(|_| ProgramError::AccountDataTooSmall)?,
    );
    let next = current
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    data[..8].copy_from_slice(&next.to_le_bytes());
    msg!("veto-demo-target counter={}", next);
    Ok(())
}
