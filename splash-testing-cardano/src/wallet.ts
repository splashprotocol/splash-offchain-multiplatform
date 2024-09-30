import {fromHex, Lucid} from '@lucid-evolution/lucid';
import * as CML from '@anastasia-labs/cardano-multiplatform-lib-nodejs';
import { mnemonicToEntropy } from "bip39"

export async function setupWallet(lucid: Lucid) {
  const seed = await Deno.readTextFile('./seed.txt');
  return lucid.selectWallet.fromSeed(seed);
}

export async function getPrivateKey() {
  const key = await Deno.readTextFile('./privateKey.txt');
  return CML.Bip32PrivateKey.from_bech32(key);
}

