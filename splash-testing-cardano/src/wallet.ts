import { Lucid } from 'https://deno.land/x/lucid@0.10.7/mod.ts';
import { getLucid } from "./lucid.ts";


export async function createWallet(lucid: Lucid) {
    const seed = lucid.utils.generateSeedPhrase();
    console.log(`New seed!: ${seed}`);
}

export async function setupWallet(lucid: Lucid) {
    const seed = "naive happy tell transfer iron snow slim humble addict improve food hurdle ride fiscal win evolve response vault excess ten option hamster law lyrics"
    lucid.selectWalletFromSeed(seed);
  }

export async function main() {
    const lucid = await getLucid();
    await createWallet(lucid);
}
// main();