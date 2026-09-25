use {
    anyhow::{anyhow, Context, Result},
    base64::{engine::general_purpose::STANDARD as BASE64, Engine as _},
    solana_sdk::{signature::read_keypair_file, transaction::Transaction},
    std::env,
};

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
