import {Blockfrost, Lucid, LucidEvolution} from '@lucid-evolution/lucid';

export async function getLucid(): Promise<LucidEvolution> {
  const token = await Deno.readTextFile('./token.txt');
  return Lucid(
    new Blockfrost('https://cardano-preprod.blockfrost.io/api/v0', token),
    'Preprod',
  );
}
