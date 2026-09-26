use {
    anyhow::{anyhow, bail, Context, Result},
    base64::{engine::general_purpose::STANDARD as BASE64, Engine as _},
    solana_sdk::{
        pubkey::Pubkey, signature::read_keypair_file, signer::Signer, transaction::Transaction,
    },
    std::env,
};

/// An approval only ever needs the operator as a read-only signer. Refusing
/// any transaction where the operator pays fees or is writable means signing
/// an approval can never move the operator's funds or alter its accounts.
fn check_operator_role(transaction: &Transaction, operator: &Pubkey) -> Result<()> {
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
    Ok(())
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let keypair_path = args.next().ok_or_else(|| {
        anyhow!("usage: external_operator_signer <keypair-path> <base64-transaction>")
    })?;
    let encoded = args
        .next()
        .ok_or_else(|| anyhow!("missing base64 transaction"))?;
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
    check_operator_role(&transaction, &operator.pubkey())?;
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
        super::check_operator_role,
        solana_sdk::{
            hash::Hash,
            instruction::{AccountMeta, Instruction},
            message::Message,
            pubkey::Pubkey,
            transaction::Transaction,
        },
        solana_system_interface::instruction as system_instruction,
    };

    fn transaction(instructions: &[Instruction], payer: &Pubkey) -> Transaction {
        let mut tx = Transaction::new_unsigned(Message::new(instructions, Some(payer)));
        tx.message.recent_blockhash = Hash::default();
        tx
    }

    #[test]
    fn accepts_the_operator_as_a_read_only_signer() {
        let (operator, payer) = (Pubkey::new_unique(), Pubkey::new_unique());
        let approval = Instruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new_readonly(operator, true),
            ],
            data: vec![1],
        };
        assert!(check_operator_role(&transaction(&[approval], &payer), &operator).is_ok());
    }

    #[test]
    fn refuses_a_transfer_out_of_the_operator_wallet() {
        let (operator, payer) = (Pubkey::new_unique(), Pubkey::new_unique());
        let drain = system_instruction::transfer(&operator, &Pubkey::new_unique(), 1);
        let error = check_operator_role(&transaction(&[drain], &payer), &operator).unwrap_err();
        assert!(error.to_string().contains("writable"));
    }

    #[test]
    fn refuses_the_operator_as_fee_payer() {
        let operator = Pubkey::new_unique();
        let noop = Instruction { program_id: Pubkey::new_unique(), accounts: vec![], data: vec![] };
        let error = check_operator_role(&transaction(&[noop], &operator), &operator).unwrap_err();
        assert!(error.to_string().contains("fee"));
    }
}
