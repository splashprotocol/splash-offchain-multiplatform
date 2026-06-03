const assetSchema = {
  title: "Asset",
  dataType: "constructor",
  index: 0,
  fields: [
    { dataType: "bytes", title: "policy" },
    { dataType: "bytes", title: "name" },
  ],
} as const;

const rationalSchema = {
  title: "Rational",
  dataType: "constructor",
  index: 0,
  fields: [
    { dataType: "integer", title: "num" },
    { dataType: "integer", title: "denom" },
  ],
} as const;

const credentialSchema = {
  title: "Credential",
  anyOf: [
    {
      title: "VerificationKeyCredential",
      dataType: "constructor",
      index: 0,
      fields: [{ dataType: "bytes" }],
    },
    {
      title: "ScriptCredential",
      dataType: "constructor",
      index: 1,
      fields: [{ dataType: "bytes" }],
    },
  ],
} as const;

const referencedSchema = {
  title: "Referenced",
  anyOf: [
    {
      title: "Inline",
      dataType: "constructor",
      index: 0,
      fields: [credentialSchema],
    },
    {
      title: "Pointer",
      dataType: "constructor",
      index: 1,
      fields: [
        { dataType: "integer", title: "slotNumber" },
        { dataType: "integer", title: "transactionIndex" },
        { dataType: "integer", title: "certificateIndex" },
      ],
    },
  ],
} as const;

export const AuctionAuction: { conf: any } = {
  conf: {
    title: "Config",
    anyOf: [{
      title: "Config",
      dataType: "constructor",
      index: 0,
      fields: [
        { title: "base", anyOf: [assetSchema] },
        { title: "quote", anyOf: [assetSchema] },
        { title: "priceStart", anyOf: [rationalSchema] },
        { dataType: "integer", title: "startTime" },
        { dataType: "integer", title: "stepLen" },
        { dataType: "integer", title: "steps" },
        { dataType: "integer", title: "priceDacayNum" },
        { title: "feePerQuote", anyOf: [rationalSchema] },
        { dataType: "bytes", title: "redeemer" },
      ],
    }],
  },
};

export const LimitOrderLimitOrder: { conf: any } = {
  conf: {
    title: "LimitOrderConfig",
    anyOf: [{
      title: "LimitOrderConfig",
      dataType: "constructor",
      index: 0,
      fields: [
        { dataType: "bytes", title: "tag" },
        { dataType: "bytes", title: "beacon" },
        { title: "input", anyOf: [assetSchema] },
        { dataType: "integer", title: "tradableInput" },
        { dataType: "integer", title: "costPerExStep" },
        { dataType: "integer", title: "minMarginalOutput" },
        { title: "output", anyOf: [assetSchema] },
        { title: "basePrice", anyOf: [rationalSchema] },
        { dataType: "integer", title: "fee" },
        {
          title: "redeemerAddress",
          anyOf: [{
            title: "Address",
            dataType: "constructor",
            index: 0,
            fields: [
              { ...credentialSchema, title: "paymentCredential" },
              {
                title: "stakeCredential",
                anyOf: [
                  {
                    title: "Some",
                    dataType: "constructor",
                    index: 0,
                    fields: [referencedSchema],
                  },
                  {
                    title: "None",
                    dataType: "constructor",
                    index: 1,
                    fields: [],
                  },
                ],
              },
            ],
          }],
        },
        { dataType: "bytes", title: "cancellationPkh" },
        {
          dataType: "list",
          items: { dataType: "bytes" },
          title: "permittedExecutors",
        },
      ],
    }],
  },
};

export const ClassicCfmmPool: { conf: any } = {
  conf: {
    title: "LegacyCFMMPoolConfig",
    anyOf: [{
      title: "LegacyCFMMPoolConfig",
      dataType: "constructor",
      index: 0,
      fields: [
        { title: "poolNft", anyOf: [assetSchema] },
        { title: "assetX", anyOf: [assetSchema] },
        { title: "assetY", anyOf: [assetSchema] },
        { title: "assetLq", anyOf: [assetSchema] },
        { dataType: "integer", title: "lpFeeNum" },
        { dataType: "integer", title: "treasuryFeeNum" },
        { dataType: "integer", title: "lqLowerBound" },
      ],
    }],
  },
};
