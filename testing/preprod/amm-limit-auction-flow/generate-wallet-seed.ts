import { generateMnemonic } from "npm:bip39@3.1.0";

const seedPath = Deno.args[0];
if (!seedPath) {
  throw new Error("Usage: generate-wallet-seed.ts PATH");
}

const seed = generateMnemonic(256);
await Deno.writeTextFile(seedPath, `${seed}\n`, { mode: 0o600 });
