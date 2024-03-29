import { Data } from "https://deno.land/x/lucid@0.10.7/mod.ts";

const BalancePoolDatumSchema = Data.Object({
    poolNft: Data.Bytes(),
    amount: Data.Integer(),
    private: Data.Boolean(),
});