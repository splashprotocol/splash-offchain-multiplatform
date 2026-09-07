import {Blockfrost, Lucid, LucidEvolution} from '@lucid-evolution/lucid';

export async function getLucid(): Promise<LucidEvolution> {
  const token = await Deno.readTextFile('./token.txt');

  return Lucid(
    new Blockfrost('https://cardano-mainnet.blockfrost.io/api/v0', token),
    'Mainnet',
  );
}