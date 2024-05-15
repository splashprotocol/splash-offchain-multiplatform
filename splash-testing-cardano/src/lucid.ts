import { Blockfrost, Lucid } from 'https://deno.land/x/lucid@0.10.7/mod.ts';

export async function getLucid() {
  const token = "mainneterNVjdoJUvv9PrWBz9sXxG8tHrNbtvwl";
  return Lucid.new(
    new Blockfrost('https://cardano-mainnet.blockfrost.io/api/v0', token),
    'Mainnet',
  );
}
