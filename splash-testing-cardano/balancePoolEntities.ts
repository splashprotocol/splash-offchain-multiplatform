import { Data } from "@lucid-evolution/lucid";

const BalancePoolDatumSchema = Data.Object({
    poolNft: Data.Bytes(),
    amount: Data.Integer(),
    private: Data.Boolean(),
});