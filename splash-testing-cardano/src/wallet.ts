import { Lucid } from 'https://deno.land/x/lucid@0.10.7/mod.ts';

export async function setupWallet(lucid: Lucid) {
  const seed = await Deno.readTextFile('./seed.txt');
  lucid.selectWalletFromSeed(seed);
}

