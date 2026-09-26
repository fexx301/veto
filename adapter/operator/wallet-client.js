import { Transaction } from "@solana/web3.js";

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

// An approval only ever needs the operator as a read-only signer. Refuse any
// transaction where the operator pays fees or is writable, so signing an
// approval can never move the operator's funds or alter its accounts.
export function checkOperatorRole(transaction, operator) {
  if (transaction.feePayer?.toString() === operator) {
    throw new Error("Refusing to sign: the operator would pay the transaction fee.");
  }
  const message = transaction.compileMessage();
  const index = message.accountKeys.findIndex((key) => key.toString() === operator);
  if (index < 0) throw new Error("Refusing to sign: the operator is not part of this transaction.");
  if (message.isAccountWritable(index)) {
    throw new Error("Refusing to sign: the operator account would be writable.");
  }
}

export async function signApproval(encoded, expectedOperator) {
  const provider = injectedProvider();
  const connection = await provider.connect();
  const publicKey = connection.publicKey || provider.publicKey;
  if (!publicKey || publicKey.toString() !== expectedOperator) {
    throw new Error(`Connect the configured operator wallet ${expectedOperator}.`);
  }
  const transaction = deserializeTransaction(encoded);
  checkOperatorRole(transaction, expectedOperator);
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
