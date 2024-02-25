import { Lucid, toHex } from "https://deno.land/x/lucid@0.10.7/mod.ts";
import { walletFromSeed } from "https://deno.land/x/lucid@0.10.7/src/misc/wallet.ts";
import { C } from "https://deno.land/x/lucid@0.10.7/mod.ts";

export async function setupWallet(lucid: Lucid) {
  const seed = await Deno.readTextFile("./seed.txt");
  const { paymentKey } = walletFromSeed(
    seed,
    {
      addressType: "Base",
      accountIndex: 0,
    },
  );

  const pubKey = toHex(
    C.PrivateKey.from_bech32(paymentKey).to_public().as_bytes(),
  );
  lucid.selectWalletFromSeed(seed);
  return pubKey;
}
