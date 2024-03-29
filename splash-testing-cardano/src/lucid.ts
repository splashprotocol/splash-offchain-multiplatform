import { Blockfrost, Lucid } from 'https://deno.land/x/lucid@0.10.7/mod.ts';

export async function getLucid() {
  const token = "preprodnKAerL1esHtX6ptiwRQjOL9grxCTj5Hf";
  return Lucid.new(
    new Blockfrost('https://cardano-preprod.blockfrost.io/api/v0', token),
    'Preprod',
  );
}
