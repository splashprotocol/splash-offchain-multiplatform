import { Blockfrost, Lucid } from 'https://deno.land/x/lucid@0.10.7/mod.ts';

export async function getLucid() {
  //const token = await Deno.readTextFile('./token.txt');
  return Lucid.new(
    new Blockfrost('https://cardano-preprod.blockfrost.io/api/v0', "preprodSp3kLj4keV8ooa3RMYgQLFMf5KY5wQo9"),
    'Preprod',
  );
}
