import { Blockfrost, Lucid } from 'https://deno.land/x/lucid@0.10.7/mod.ts';

export async function getLucid() {
  const token = "preprod5w7d2sFOTsIH8xxepGXxv2FZmqkGAqhi";
  return Lucid.new(
    new Blockfrost('https://cardano-preprod.blockfrost.io/api/v0', token),
    'Preprod',
  );
}
