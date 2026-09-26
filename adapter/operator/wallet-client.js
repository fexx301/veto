import { PublicKey, Transaction } from "@solana/web3.js";

function decodeBase64(encoded) {
  const binary = atob(encoded);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

function encodeBase64(bytes) {
  let binary = "";
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(binary);
}

function injectedProvider() {
  const provider = window.solana;
  if (!provider || typeof provider.connect !== "function" || typeof provider.signTransaction !== "function") {
    throw new Error("No compatible injected Solana wallet was found.");
  }
  return provider;
}

export function deserializeTransaction(encoded) {
  return Transaction.from(decodeBase64(encoded));
}

export function serializeTransaction(transaction, requireAllSignatures = true) {
  return encodeBase64(transaction.serialize({
    requireAllSignatures,
    verifySignatures: requireAllSignatures,
  }));
}

const SWIG_PROGRAM = "swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB";
const SYSTEM_PROGRAM = "11111111111111111111111111111111";

// Accepts only what a Veto approval contains (same rules as the standalone
// signer): the operator is a read-only signer that does not pay the fee; gate
// instructions are initialize (0), reapprove (1) or approve-program (3) with
// the operator only as account 1; a Swig instruction must be AddAuthorityV1
// by the root role adding a ProgramExec role bound to the gate, with the
// operator only as account 3; System instructions must not involve the
// operator; no other program may appear. A read-only signer can still
// authorize a token transfer, so the instructions themselves are checked.
export function checkApproval(transaction, operator, gate) {
  if (!gate) throw new Error("Refusing to sign: the Veto program id is missing.");
  if (transaction.feePayer?.toString() === operator) {
    throw new Error("Refusing to sign: the operator would pay the transaction fee.");
  }
  const message = transaction.compileMessage();
  const keys = message.accountKeys.map((key) => key.toString());
  const index = keys.indexOf(operator);
  if (index < 0 || !message.isAccountSigner(index)) {
    throw new Error("Refusing to sign: the operator is not a required signer.");
  }
  if (message.isAccountWritable(index)) {
    throw new Error("Refusing to sign: the operator account would be writable.");
  }
  for (const instruction of transaction.instructions) {
    const program = instruction.programId.toString();
    const data = Uint8Array.from(instruction.data);
    const positions = instruction.keys
      .map((meta, position) => (meta.pubkey.toString() === operator ? position : -1))
      .filter((position) => position >= 0);
    const onlyAt = (position) => positions.every((p) => p === position);
    if (program === gate) {
      if (![0, 1, 3].includes(data[0]) || !onlyAt(1)) throw new Error("Refusing to sign: unexpected Veto instruction.");
    } else if (program === SWIG_PROGRAM) {
      const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
      const bound = data.length >= 48 ? new PublicKey(data.slice(16, 48)).toString() : "";
      if (data.length < 48 || view.getUint16(0, true) !== 1 || view.getUint16(6, true) !== 7
          || view.getUint32(12, true) !== 0 || bound !== gate || !onlyAt(3)) {
        throw new Error("Refusing to sign: only adding a Veto-bound ProgramExec role is allowed.");
      }
    } else if (program === SYSTEM_PROGRAM) {
      if (positions.length) throw new Error("Refusing to sign: a System instruction would involve the operator.");
    } else {
      throw new Error(`Refusing to sign: unexpected program ${program}.`);
    }
  }
}

export async function signApproval(encoded, expectedOperator, expectedGate) {
  const provider = injectedProvider();
  const connection = await provider.connect();
  const publicKey = connection.publicKey || provider.publicKey;
  if (!publicKey || publicKey.toString() !== expectedOperator) {
    throw new Error(`Connect the configured operator wallet ${expectedOperator}.`);
  }
  const transaction = deserializeTransaction(encoded);
  checkApproval(transaction, expectedOperator, expectedGate);
  const signed = await provider.signTransaction(transaction);
  return serializeTransaction(signed);
}

export async function connectWallet() {
  const provider = injectedProvider();
  const connection = await provider.connect();
  const publicKey = connection.publicKey || provider.publicKey;
  if (!publicKey) throw new Error("The wallet did not return a public key.");
  return publicKey.toString();
}
