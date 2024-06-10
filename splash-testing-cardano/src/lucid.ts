import { Blockfrost, Lucid } from 'https://deno.land/x/lucid@0.10.7/mod.ts';

export async function getLucid() {
  //const token = await Deno.readTextFile('./token.txt');
  return Lucid.new(
    new Blockfrost('https://cardano-mainnet.blockfrost.io/api/v0', "mainnetJ5Bx2anaNVhu1Z9KotBINVtQf43LNrdb"),
    'Mainnet',
  );
}
