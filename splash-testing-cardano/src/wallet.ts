import { Lucid } from 'https://deno.land/x/lucid@0.10.7/mod.ts';

export async function setupWallet(lucid: Lucid) {
  const seed = "uncle inherit chest series fox entry vague basic slab grunt carbon collect foot half purse usual dwarf you fuel sunset pull swamp gain diet";
  lucid.selectWalletFromSeed(seed);
}

