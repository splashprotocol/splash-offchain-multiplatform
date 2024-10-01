import { Blockfrost, Lucid } from '@lucid-evolution/lucid';

export async function getLucid() {
  const token = await Deno.readTextFile('./token.txt');
  return Lucid(
    new Blockfrost('https://cardano-preprod.blockfrost.io/api/v0', token),
    'Preprod',
  );
}
