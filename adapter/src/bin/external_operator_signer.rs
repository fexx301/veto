use {
    anyhow::{anyhow, bail, Context, Result},
    base64::{engine::general_purpose::STANDARD as BASE64, Engine as _},
    solana_sdk::{
        pubkey::Pubkey, signature::read_keypair_file, signer::Signer, transaction::Transaction,
    },
    std::env,
};

const SWIG_PROGRAM: Pubkey = solana_sdk::pubkey!("swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB");
const SYSTEM_PROGRAM: Pubkey = solana_sdk::pubkey!("11111111111111111111111111111111");
/// Swig `AddAuthorityV1` (u16 instruction 1) with a `ProgramExec` (type 7)
/// authority, acting as the root role (0).
const SWIG_ADD_AUTHORITY: u16 = 1;
const PROGRAM_EXEC: u16 = 7;

/// Accepts only what a Veto approval contains, and refuses anything else the
/// operator's signature could authorize:
///
/// - the operator is a required, read-only signer and does not pay the fee;
/// - Veto gate instructions are initialize (0), reapprove (1) or
///   approve-program (3), with the operator only as the authority (account 1);
/// - a Swig instruction must be `AddAuthorityV1` by the root role, adding a
///   `ProgramExec` role bound to the gate, with the operator only as the
///   acting authority (account 3);
/// - System instructions (the server funding the policy account) must not
///   involve the operator;
/// - no other program may appear.
///
/// A read-only signer can still authorize a token transfer, so checking
/// writability alone is not enough; the instructions themselves are checked.
fn check_approval(transaction: &Transaction, operator: &Pubkey, gate: &Pubkey) -> Result<()> {
    let message = &transaction.message;
    let keys = &message.account_keys;
    let header = &message.header;
    let signed = header.num_required_signatures as usize;
    let index = keys
        .iter()
        .position(|key| key == operator)
        .filter(|index| *index < signed)
        .ok_or_else(|| anyhow!("this key is not a required signer for this transaction"))?;
    if index == 0 {
        bail!("refusing to sign: the operator would pay the transaction fee")
    }
    let writable = if index < signed {
        index < signed - header.num_readonly_signed_accounts as usize
    } else {
        index < keys.len() - header.num_readonly_unsigned_accounts as usize
    };
    if writable {
        bail!("refusing to sign: the operator account would be writable")
    }
    for instruction in &message.instructions {
        let program = keys
            .get(instruction.program_id_index as usize)
            .ok_or_else(|| anyhow!("malformed instruction"))?;
        let data = &instruction.data;
        let operator_positions: Vec<usize> = instruction
            .accounts
            .iter()
            .enumerate()
            .filter(|(_, key_index)| keys.get(**key_index as usize) == Some(operator))
            .map(|(position, _)| position)
            .collect();
        let operator_only_at = |position: usize| operator_positions.iter().all(|p| *p == position);
        if program == gate {
            if !matches!(data.first(), Some(0 | 1 | 3)) || !operator_only_at(1) {
                bail!("refusing to sign: unexpected Veto instruction")
            }
        } else if *program == SWIG_PROGRAM {
            let u16_at = |offset: usize| data.get(offset..offset + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
            let acting_role = data.get(12..16).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            let bound_program = data.get(16..48);
            if u16_at(0) != Some(SWIG_ADD_AUTHORITY)
                || u16_at(6) != Some(PROGRAM_EXEC)
                || acting_role != Some(0)
                || bound_program != Some(gate.as_ref())
                || !operator_only_at(3)
            {
                bail!("refusing to sign: only adding a Veto-bound ProgramExec role is allowed")
            }
        } else if *program == SYSTEM_PROGRAM {
            if !operator_positions.is_empty() {
                bail!("refusing to sign: a System instruction would involve the operator")
            }
        } else {
            bail!("refusing to sign: unexpected program {program}")
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let keypair_path = args.next().ok_or_else(|| {
        anyhow!("usage: external_operator_signer <keypair-path> <base64-transaction> <veto-gate-program-id>")
    })?;
    let encoded = args
        .next()
        .ok_or_else(|| anyhow!("missing base64 transaction"))?;
    let gate: Pubkey = args
        .next()
        .ok_or_else(|| anyhow!("missing Veto gate program id"))?
        .parse()
        .context("invalid Veto gate program id")?;
    if args.next().is_some() {
        return Err(anyhow!("unexpected extra argument"));
    }
    let operator = read_keypair_file(&keypair_path)
        .map_err(|error| anyhow!(error.to_string()))
        .with_context(|| format!("could not read operator keypair {keypair_path}"))?;
    let bytes = BASE64
        .decode(encoded)
        .context("transaction is not valid base64")?;
    let mut transaction: Transaction =
        bincode::deserialize(&bytes).context("transaction is malformed")?;
    check_approval(&transaction, &operator.pubkey(), &gate)?;
    let blockhash = transaction.message.recent_blockhash;
    transaction
        .try_partial_sign(&[&operator], blockhash)
        .context("operator is not a required signer for this transaction")?;
    transaction
        .verify()
        .context("transaction is not fully signed after operator approval")?;
    println!("{}", BASE64.encode(bincode::serialize(&transaction)?));
    Ok(())
}

#[cfg(test)]
mod tests {
    use {
        super::{check_approval, SWIG_PROGRAM},
        solana_sdk::{
            hash::Hash,
            instruction::{AccountMeta, Instruction},
            message::Message,
            pubkey::Pubkey,
            transaction::Transaction,
        },
        solana_system_interface::instruction as system_instruction,
    };

    const TOKEN_PROGRAM: Pubkey = solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

    fn transaction(instructions: &[Instruction], payer: &Pubkey) -> Transaction {
        let mut tx = Transaction::new_unsigned(Message::new(instructions, Some(payer)));
        tx.message.recent_blockhash = Hash::default();
        tx
    }

    fn gate_approval(gate: Pubkey, operator: Pubkey, discriminator: u8) -> Instruction {
        Instruction {
            program_id: gate,
            accounts: vec![
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new_readonly(operator, true),
            ],
            data: vec![discriminator, 0, 0, 0, 0, 0, 0, 0, 0],
        }
    }

    /// Swig AddAuthorityV1 by role 0 adding an authority of `authority_type`
    /// bound to `bound_program`, with the operator as the acting authority.
    fn swig_add_authority(operator: Pubkey, authority_type: u16, bound_program: Pubkey) -> Instruction {
        let mut data = vec![0u8; 16];
        data[0..2].copy_from_slice(&1u16.to_le_bytes());
        data[6..8].copy_from_slice(&authority_type.to_le_bytes());
        data.extend_from_slice(bound_program.as_ref());
        data.extend_from_slice(&[0u8; 48]);
        Instruction {
            program_id: SWIG_PROGRAM,
            accounts: vec![
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new(Pubkey::new_unique(), true),
                AccountMeta::new_readonly(solana_sdk::pubkey!("11111111111111111111111111111111"), false),
                AccountMeta::new_readonly(operator, true),
            ],
            data,
        }
    }

    fn keys() -> (Pubkey, Pubkey, Pubkey) {
        (Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique())
    }

    #[test]
    fn accepts_the_setup_approval() {
        let (operator, payer, gate) = keys();
        let setup = [
            system_instruction::create_account(&payer, &Pubkey::new_unique(), 1, 522, &gate),
            gate_approval(gate, operator, 0),
            swig_add_authority(operator, 7, gate),
            gate_approval(gate, operator, 3),
        ];
        assert!(check_approval(&transaction(&setup, &payer), &operator, &gate).is_ok());
    }

    #[test]
    fn accepts_a_reapproval() {
        let (operator, payer, gate) = keys();
        let tx = transaction(&[gate_approval(gate, operator, 1)], &payer);
        assert!(check_approval(&tx, &operator, &gate).is_ok());
    }

    #[test]
    fn refuses_a_token_transfer_signed_by_the_operator_as_owner() {
        let (operator, payer, gate) = keys();
        // SPL Token Transfer (3): source, destination, owner (read-only signer).
        let transfer = Instruction {
            program_id: TOKEN_PROGRAM,
            accounts: vec![
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new_readonly(operator, true),
            ],
            data: [vec![3], 1_000u64.to_le_bytes().to_vec()].concat(),
        };
        let tx = transaction(&[gate_approval(gate, operator, 1), transfer], &payer);
        assert!(check_approval(&tx, &operator, &gate).unwrap_err().to_string().contains("unexpected program"));
    }

    #[test]
    fn refuses_adding_a_non_veto_role_as_root() {
        let (operator, payer, gate) = keys();
        // An Ed25519 (type 1) role would give some other key control of the wallet.
        let tx = transaction(&[swig_add_authority(operator, 1, Pubkey::new_unique())], &payer);
        assert!(check_approval(&tx, &operator, &gate).is_err());
        // A ProgramExec role bound to a different program is refused too.
        let tx = transaction(&[swig_add_authority(operator, 7, Pubkey::new_unique())], &payer);
        assert!(check_approval(&tx, &operator, &gate).is_err());
    }

    #[test]
    fn refuses_a_transfer_out_of_the_operator_wallet() {
        let (operator, payer, gate) = keys();
        let drain = system_instruction::transfer(&operator, &Pubkey::new_unique(), 1);
        assert!(check_approval(&transaction(&[drain], &payer), &operator, &gate).is_err());
    }

    #[test]
    fn refuses_other_gate_instructions_and_the_operator_as_fee_payer() {
        let (operator, payer, gate) = keys();
        let authorize = gate_approval(gate, operator, 2);
        assert!(check_approval(&transaction(&[authorize], &payer), &operator, &gate).is_err());
        let tx = transaction(&[gate_approval(gate, operator, 1)], &operator);
        assert!(check_approval(&tx, &operator, &gate).unwrap_err().to_string().contains("fee"));
    }
}
