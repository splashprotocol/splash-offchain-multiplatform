// deno-lint-ignore-file
import {
  applyDoubleCborEncoding,
  applyParamsToScript,
  Data,
  Validator,
} from "@lucid-evolution/lucid";

export interface IBeaconBeacon {
  new (
    ref: { transactionId: { hash: string }; outputIndex: bigint },
  ): Validator;
  _: Data;
}

export const BeaconBeacon = Object.assign(
  function (ref: { transactionId: { hash: string }; outputIndex: bigint }) {
    return {
      type: "PlutusV2",
      script: applyParamsToScript(
        applyDoubleCborEncoding(
          "5901e301000032323232323232222533300432533233006300130073754004264a6646601066e212000332232533300b3370e900118061baa0011480004dd6980818069baa00132533300b3370e900118061baa00114c103d87a80001323300100137566022601c6ea8008894ccc040004530103d87a8000132323253330103371e911010100375c602200626012660286ea00052f5c026600a00a0046eb4c044008c050008c048004c8cc00400400c894ccc03c0045300103d87a80001323232533300f3371e00c6eb8c04000c4c020cc04cdd3000a5eb804cc014014008dd598080011809801180880099198008009bab300e300f300f300f300f300b3754600660166ea8018894ccc03400452f5bded8c0264646464a66601c66e3d2201000021003133012337606ea4008dd3000998030030019bab300f003375c601a0046022004601e0026eb8c034c028dd50020992999804980218051baa00113375e600660166ea8c038c02cdd50008040b1999180080091129998070010a6103d87a800013232533300d300800313006330110024bd70099980280280099b8000348004c04800cc040008dd6180118051baa3002300a375400a90001ba548000528918060009b874800058c024c028c018dd50008a4c26cacae6955ceaab9e5573eae815d0aba201",
        ),
        [ref],
        {
          "dataType": "list",
          "items": [{
            "title": "OutputReference",
            "description":
              "An `OutputReference` is a unique reference to an output on-chain. The `output_index`\n corresponds to the position in the output list of the transaction (identified by its id)\n that produced that output",
            "anyOf": [{
              "title": "OutputReference",
              "dataType": "constructor",
              "index": 0,
              "fields": [{
                "title": "transactionId",
                "description":
                  "A unique transaction identifier, as the hash of a transaction body. Note that the transaction id\n isn't a direct hash of the `Transaction` as visible on-chain. Rather, they correspond to hash\n digests of transaction body as they are serialized on the network.",
                "anyOf": [{
                  "title": "TransactionId",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{ "dataType": "bytes", "title": "hash" }],
                }],
              }, { "dataType": "integer", "title": "outputIndex" }],
            }],
          }],
        },
      ),
    };
  },
  { _: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IBeaconBeacon;

export interface ILiquidityLockerLiquidityLocker {
  new (): Validator;
  conf: { lockedUntil: bigint; redeemer: string };
  successorIx: bigint;
}

export const LiquidityLockerLiquidityLocker = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "5904040100003232323232323223232322322533300832323253323300c3001300d37540042646464a66601e601a60206ea80044c8c8c8c94ccc04cc044c050dd500089919299980a9991191980080080191299980e0008a50132533301a3371e6eb8c07c008010528899801801800980f8009bac301a301b301b301b301b301b301b301b301b301737546012602e6ea8038dd71803180b9baa014100114a0a66602866ebcc020c058dd50021804180b1baa00113253330153370e9002180b1baa00113232323253330193233001001323300100137566018603a6ea802c894ccc07c00452f5c0264666444646600200200644a66604a00220062646604e6e9ccc09cdd4803198139ba9375c60480026604e6ea0dd69812800a5eb80cc00c00cc0a4008c09c004dd7180f0009bab301f001330030033023002302100122533301e00114a2264a666038646466e24dd6981198120009991192999810980b18111baa0011480004dd6981318119baa0013253330213016302237540022980103d87a8000132330010013756604e60486ea8008894ccc098004530103d87a8000132323253330263371e00e6eb8c09c00c4c064cc0a8dd4000a5eb804cc014014008dd698138011815001181400099198008008051129998128008a6103d87a8000132323253330253371e00e6eb8c09800c4c060cc0a4dd3000a5eb804cc014014008dd59813001181480118138009bae3023002375c604600260460026eb0c0840084cc00c00c004528181080088008a50337126eb4c030c068dd500b9bad300c301a37540066eacc020c064dd50021809800980d180b9baa001163003301637540022664464a66602e601860306ea80044c94ccc060c94ccc070c06c00454ccc064c038c0680045288a99980c980b980d0008a5016163754601260346ea8c030c068dd5002099b880030011337120060026eb4c070c064dd50008a50300a30183754601460306ea8008c064c068c068c068c068c068c068c068c058dd50059bad3008301637540266030602a6ea800458ccc8c0040048894ccc0600085300103d87a800013232533301730150031300a3301b0024bd70099980280280099b8000348004c07000cc068008dd61800980a1baa00900c2301730183018001300130123754602a60246ea80088c054c05800458cc88c8cc00400400c894ccc054004530103d87a80001323253330143375e6010602c6ea80080144c01ccc0600092f5c02660080080026032004602e0026eb0c008c040dd5002980998081baa004374a9000118090009b874800858c03cc040008c038004c028dd50008a4c26cac6eb4004c00400c94ccc010c008c014dd5000899191919299980598070010a4c2c6eb8c030004c030008dd6980500098031baa00116370e90002b9a5573aaae7955cfaba05742ae89",
    };
  },
  {
    conf: {
      "title": "Config",
      "anyOf": [{
        "title": "Config",
        "dataType": "constructor",
        "index": 0,
        "fields": [{ "dataType": "integer", "title": "lockedUntil" }, {
          "dataType": "bytes",
          "title": "redeemer",
        }],
      }],
    },
  },
  { successorIx: { "dataType": "integer" } },
) as unknown as ILiquidityLockerLiquidityLocker;

export interface IAuctionAuction {
  new (): Validator;
  conf: {
    base: { policy: string; name: string };
    quote: { policy: string; name: string };
    priceStart: { num: bigint; denom: bigint };
    startTime: bigint;
    stepLen: bigint;
    steps: bigint;
    priceDacayNum: bigint;
    feePerQuote: { num: bigint; denom: bigint };
    redeemer: string;
  };
  action: { Exec: { spanIx: bigint; successorIx: bigint } } | "Cancel";
}

export const AuctionAuction = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "590717010000323232323232322323223232253330083232533300a3008300b375400c2646464646464a666020601660226ea80044c8c94ccc048c040c04cdd5000899191919191919191919299980e180d180e9baa001132323232323232323232323232323232323232533302f0171533302f0011533302f0051533302f002100614a029405280a5032533302f302d303037540022a66605e66e3cdd7181398189baa02e375c606860626ea80044c0b4034528099baf30263031375402c604c60626ea804cc094c0c0dd5181298181baa0123370e66e08018ccc004008dd6980b18179baa3013302f37540584466e0800920d00f33704a66605a010266e04cdc0806805802899b8100d00b333001002375a6048605e6ea8c04cc0bcdd50161119b82002375a6068606a606a606a606a606a606a60626ea80b8888c8ccc00400401000c8894ccc0d400840044ccc00c00cc0e0008cc010004dd6981b8011919800800a400044a66605a66e2008800452f5c02660626ea0004cc008008cdc0000a400466e2120000035333029533302900514a2200829444cdc49919b8130013756602660586ea8044c004dd5980998161baa00e233300b0014881004881000013370666e08004dd6980f98151baa002375a602260546ea80094ccc09c00c4cdc199b823370200800c6eb4c040c0a4dd500099b80375a602060526ea8004dd6980f18149baa00113370200800c602260506ea8094cdc78042441003371e00c91010033300437566018604a6ea801c014dd7180618129baa300c302537540446660066eacc02cc090dd50030029bae300b30243754603260486ea8084ccc008dd5980518119baa008003375c601460466ea8c028c08cdd50101998009bab30093022375400e0066eb8c024c088dd5180b98111baa01f222325333023301e302437540022900009bad30283025375400264a666046603c60486ea80045300103d87a80001323300100137566052604c6ea8008894ccc0a0004530103d87a8000132323253330283371e00e6eb8c0a400c4c060cc0b0dd4000a5eb804cc014014008dd698148011816001181500099198008008021129998138008a6103d87a8000132323253330273371e00e6eb8c0a000c4c05ccc0acdd3000a5eb804cc014014008dd59814001181580118148009bae301530203754600e60406ea8074dd7180a180f9baa3014301f37540386042603c6ea800458ccc8c0040048894ccc0840085300103d87a8000132325333020301e00313010330240024bd70099980280280099b8000348004c09400cc08c008dd61800980e9baa00d00f23020302130210013002301b3754603c60366ea80214ccc060cdc40069bad301d301e301e301e301e301e301a375402e2a66603064a666032602860346ea80044c94ccc068c94ccc078c07400454ccc06cc058c0700045288a99980d980c980e0008a5016163754600660386ea8c044c070dd5002099b8800700113371200e0026eb4c078c06cdd50008a50300f301a3754601e60346ea80084c94ccc064c050c068dd5000899299980d19299980f180e8008a99980d980b180e0008a511533301b3019301c00114a02c2c6ea8c00cc070dd51801980e1baa00413371000200c266e24004018dd6980f180d9baa00114a0601e60346ea8c004c068dd50010a5014a04603a603c002600260306ea80208c06cc070c070c070c070c070c070c070004cdc000080119b80375a6030603260326032602a6ea8048cdc10008041bad30173018301830183018301437540222c6644646600200200644a6660300022980103d87a80001323253330173375e601c60326ea80080144c01ccc06c0092f5c0266008008002603800460340026eb0c020c04cdd5001980b18099baa002374a90000b180a180a801180980098079baa006375a602260240046eb4c040004c030dd50030999119198008008019129998088008a50132533300f3371e6eb8c050008010528899801801800980a0009bac3002300c3754600260186ea800cdd7180118061baa0092300f0012300e300f300f300f300f300f300f300f300f00114984d958c94ccc01cc0140044c8c8c8c94ccc038c04400852616375a601e002601e0046eb4c034004c024dd50018a99980398010008a99980518049baa00314985858c01cdd50011b8748008c8c94ccc014c00cc018dd50020991919191919191919191919191919191919299980d180e80109919191924c602c00c602a01e602a02060280222c6eb8c06c004c06c008c064004c064008dd6980b800980b8011bad30150013015002375a602600260260046eb4c044004c044008c03c004c03c008c034004c034008c02c004c01cdd50020b12999802980198031baa001132323232533300c300f002149858dd6980680098068011bad300b001300737540022c4a6660086004600a6ea80044c8c8c8c94ccc02cc03800852616375c601800260180046eb8c028004c018dd50008b1b87480015cd2ab9d5573caae7d5d02ba157441",
    };
  },
  {
    conf: {
      "title": "Config",
      "anyOf": [{
        "title": "Config",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          {
            "title": "base",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "quote",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "priceStart",
            "anyOf": [{
              "title": "Rational",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "integer", "title": "num" }, {
                "dataType": "integer",
                "title": "denom",
              }],
            }],
          },
          { "dataType": "integer", "title": "startTime" },
          { "dataType": "integer", "title": "stepLen" },
          { "dataType": "integer", "title": "steps" },
          { "dataType": "integer", "title": "priceDacayNum" },
          {
            "title": "feePerQuote",
            "anyOf": [{
              "title": "Rational",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "integer", "title": "num" }, {
                "dataType": "integer",
                "title": "denom",
              }],
            }],
          },
          { "dataType": "bytes", "title": "redeemer" },
        ],
      }],
    },
  },
  {
    action: {
      "title": "Action",
      "anyOf": [{
        "title": "Exec",
        "description": "Execute order",
        "dataType": "constructor",
        "index": 0,
        "fields": [{ "dataType": "integer", "title": "spanIx" }, {
          "dataType": "integer",
          "title": "successorIx",
        }],
      }, {
        "title": "Cancel",
        "dataType": "constructor",
        "index": 1,
        "fields": [],
      }],
    },
  },
) as unknown as IAuctionAuction;

export interface IGridGridNative {
  new (): Validator;
  state: {
    beacon: string;
    token: { policy: string; name: string };
    buyShiftFactor: { num: bigint; denom: bigint };
    sellShiftFactor: { num: bigint; denom: bigint };
    maxLovelaceOffer: bigint;
    lovelaceOffer: bigint;
    price: { num: bigint; denom: bigint };
    side: boolean;
    budgetPerTransaction: bigint;
    minMarginalOutputLovelace: bigint;
    minMarginalOutputToken: bigint;
    redeemerAddress: {
      paymentCredential: { VerificationKeyCredential: [string] } | {
        ScriptCredential: [string];
      };
      stakeCredential: {
        Inline: [
          { VerificationKeyCredential: [string] } | {
            ScriptCredential: [string];
          },
        ];
      } | {
        Pointer: {
          slotNumber: bigint;
          transactionIndex: bigint;
          certificateIndex: bigint;
        };
      } | null;
    };
    cancellationPkh: string;
  };
  action: { Execute: { successorOutIndex: bigint } } | "Close";
}

export const GridGridNative = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "5908350100003232323232323223232322322533300832323232323232533300f300c3010375400a264646464a666026602260286ea80384c8c94ccc054c04cc058dd500089919299980b980a980c1baa0011323232323232323232323232323232323232323232323232323232323232323232323232533303b3370e9002181e1baa00113232533303d006100114a064a646466607e02e26464646464a66608866e2002d20001533304400415333044003100114a0294052819baf0083032330473038304537540846608e04e6608e04a6608e0466608e6ea0084cc11cdd419b8001f00a3304753330430011330073006375a6070608a6ea8094c014dd6981998229baa025101d330473330433330430014a094530103d87a80004c0103d87980003304737500326608e6ea005ccc11cdd400a99823809998239ba90124bd7019b89375a608e6090609060906090609060886ea810400ccdc49801801180200499b89012008337029000003099191919299982199b8800b4800054ccc10c01054ccc10c00c40045280a5014a066ebc01cc0c4cc118c0dcc110dd5020998230131982301219823011198231ba80203304637500406608ca66608400226600c600a6eb4c0dcc110dd501118021bad30323044375404420386608c66608400298103d87a80004c0103d87980003304637500306608c6ea0058cc118dd400a19823009198231ba90114bd70181f80599b893370466e052000008375a606a60846ea8068cdc10039bad30303042375403466e2404c018dc11bad302e304037540306e08dd69819181f9baa01722302d330423750004660846ea00052f5c06080607a6ea800458c0b4c0f0dd501199b80337026eb4c0ecc0f8dd5981d981f0109bad303b303e37566076607c04801e66e04008cccc00cc0f808c01401120063034333300201e375c607803a91010048018cccc00407400c0092006222232333001001005002222533303c3371090000008a99981f8010a40002646464a66607e66e3cdd718200018048991919299982119b8f375c608600601620022a66608a004290000a999822982400109919299982219b8f375c608a00401a2002290001bad3045001304700216375a6086004608c0046088002266600c00c00466e00011200137566080004608600660820042c6eb8c0e4c0e8008dd7181c000981a1baa3022303437540626eb8c0d8c0dc008c0d4004c0d4008dd6981980098198011bad30310013031002375a605e002605e00466e21200030293754605a002605a004605600260560046eb4c0a4004c0a4008dd698138009813801181280098128011811800981180118108009810800980e1baa019301f0013756601060346ea8004c070c064dd50008b19991800800911299980e0010a60103d87a800013232533301b30190031300a3301f0024bd70099980280280099b8000348004c08000cc07800802000cdd59802980b9baa3005301737546034602e6ea800458cc008020014dd6980c180a9baa00e132325333015301230163754002264a66602c6028602e6ea80044c8c8c94ccc06401854ccc06400840045280a503375e601a60346ea8008c074c078c078c078c078c078c078c078c078c078c078c078c068dd500b99baf300730193754600e60326ea8c070c064dd50011803980c9baa001301b0081633003009301a301737540022c6008602c6ea8034cc88c8cc00400400c894ccc068004528099299980c19b8f375c603a00400829444cc00c00c004c074004dd6180c180c980c980c980c980c980c980c980c980a9baa00a375c6030603260326032603260326032603260326032603260326032602a6ea804888c8cc00400400c894ccc064004530103d87a80001323253330183375e601a60346ea80080144c01ccc0700092f5c0266008008002603a00460360026e95200023016301700130143011375400a2c6eb0c004c040dd500291809980a180a0009bac3001300e375400646022002601e6020004601c00260146ea800452613656325333007300500113232533300c300f002149858dd6980680098049baa0021533300730040011533300a300937540042930b0b18039baa0013232533300630043007375400a26464646464646464646464646464646464646464646464646464a666046604c00426464646464932999812181118129baa007132323232533302b302e00213232498c94ccc0a8c0a00044c8c94ccc0bcc0c80084c92632533302d302b0011323253330323035002132498c0ac00458c0cc004c0bcdd50010a99981698150008991919191919299981b181c8010a4c2c6eb4c0dc004c0dc008dd6981a800981a8011bad3033001302f37540042c605a6ea800458c0c0004c0b0dd50018a99981518138008a99981698161baa00314985858c0a8dd500118120018b18160009816001181500098131baa00716301e010301d015301c0165333020301e3021375402e264646464a66604e60540042930b1bae30280013028002375c604c00260446ea805c5858dd718120009812001181100098110011bad30200013020002375a603c002603c0046eb4c070004c070008c94ccc064c06000454ccc058c04cc05c0045288a99980b180a180b8008a501616375460340026034004603000260300046eb4c058004c058008dd6980a000980a0011809000980900118080009808001180700098070011bae300c0013008375400a2c4a66600c6008600e6ea80044c8c8c8c94ccc034c04000852616375a601c002601c0046eb4c030004c020dd50008b1192999803180200089919299980598070010a4c2c6eb8c030004c020dd50010a999803180180089919299980598070010a4c2c6eb8c030004c020dd50010b18031baa001370e90011b87480015cd2ab9d5573caae7d5d02ba157441",
    };
  },
  {
    state: {
      "title": "GridStateNative",
      "anyOf": [{
        "title": "GridStateNative",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          { "dataType": "bytes", "title": "beacon" },
          {
            "title": "token",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "buyShiftFactor",
            "anyOf": [{
              "title": "Rational",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "integer", "title": "num" }, {
                "dataType": "integer",
                "title": "denom",
              }],
            }],
          },
          {
            "title": "sellShiftFactor",
            "anyOf": [{
              "title": "Rational",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "integer", "title": "num" }, {
                "dataType": "integer",
                "title": "denom",
              }],
            }],
          },
          { "dataType": "integer", "title": "maxLovelaceOffer" },
          { "dataType": "integer", "title": "lovelaceOffer" },
          {
            "title": "price",
            "anyOf": [{
              "title": "Rational",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "integer", "title": "num" }, {
                "dataType": "integer",
                "title": "denom",
              }],
            }],
          },
          {
            "title": "side",
            "anyOf": [{
              "title": "False",
              "dataType": "constructor",
              "index": 0,
              "fields": [],
            }, {
              "title": "True",
              "dataType": "constructor",
              "index": 1,
              "fields": [],
            }],
          },
          { "dataType": "integer", "title": "budgetPerTransaction" },
          { "dataType": "integer", "title": "minMarginalOutputLovelace" },
          { "dataType": "integer", "title": "minMarginalOutputToken" },
          {
            "title": "redeemerAddress",
            "description":
              "A Cardano `Address` typically holding one or two credential references.\n\n Note that legacy bootstrap addresses (a.k.a. 'Byron addresses') are\n completely excluded from Plutus contexts. Thus, from an on-chain\n perspective only exists addresses of type 00, 01, ..., 07 as detailed\n in [CIP-0019 :: Shelley Addresses](https://github.com/cardano-foundation/CIPs/tree/master/CIP-0019/#shelley-addresses).",
            "anyOf": [{
              "title": "Address",
              "dataType": "constructor",
              "index": 0,
              "fields": [{
                "title": "paymentCredential",
                "description":
                  "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                "anyOf": [{
                  "title": "VerificationKeyCredential",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{ "dataType": "bytes" }],
                }, {
                  "title": "ScriptCredential",
                  "dataType": "constructor",
                  "index": 1,
                  "fields": [{ "dataType": "bytes" }],
                }],
              }, {
                "title": "stakeCredential",
                "anyOf": [{
                  "title": "Some",
                  "description": "An optional value.",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{
                    "description":
                      "Represent a type of object that can be represented either inline (by hash)\n or via a reference (i.e. a pointer to an on-chain location).\n\n This is mainly use for capturing pointers to a stake credential\n registration certificate in the case of so-called pointer addresses.",
                    "anyOf": [{
                      "title": "Inline",
                      "dataType": "constructor",
                      "index": 0,
                      "fields": [{
                        "description":
                          "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                        "anyOf": [{
                          "title": "VerificationKeyCredential",
                          "dataType": "constructor",
                          "index": 0,
                          "fields": [{ "dataType": "bytes" }],
                        }, {
                          "title": "ScriptCredential",
                          "dataType": "constructor",
                          "index": 1,
                          "fields": [{ "dataType": "bytes" }],
                        }],
                      }],
                    }, {
                      "title": "Pointer",
                      "dataType": "constructor",
                      "index": 1,
                      "fields": [
                        { "dataType": "integer", "title": "slotNumber" },
                        { "dataType": "integer", "title": "transactionIndex" },
                        { "dataType": "integer", "title": "certificateIndex" },
                      ],
                    }],
                  }],
                }, {
                  "title": "None",
                  "description": "Nothing.",
                  "dataType": "constructor",
                  "index": 1,
                  "fields": [],
                }],
              }],
            }],
          },
          { "dataType": "bytes", "title": "cancellationPkh" },
        ],
      }],
    },
  },
  {
    action: {
      "title": "Action",
      "anyOf": [{
        "title": "Execute",
        "dataType": "constructor",
        "index": 0,
        "fields": [{ "dataType": "integer", "title": "successorOutIndex" }],
      }, {
        "title": "Close",
        "dataType": "constructor",
        "index": 1,
        "fields": [],
      }],
    },
  },
) as unknown as IGridGridNative;

export interface ILimitOrderBatchWitness {
  new (): Validator;
  _: Data;
}

export const LimitOrderBatchWitness = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "59076e01000032323232323232225333003325332330053001300637546004600e6ea800c4c8c8c8c8cccc8888c8cccc004004014011289111299980a001880089919299980b0020a501323333007007002301a0055333014004133300800300100914a060300086030008602c0066eb0c010c02cdd50019bac3001300b37540066eb0c008c02cdd5001991919191919191111919299980a9808980b1baa00113253330163375e60366eb0c060c8cdd81ba83018001374e60320026ea800530102410000132323232323232323232323232323232323232323232323232533302f53233303000313253330313371266e08024dd6981618199baa00733704a666062026266e00cdc000600400508061bad302e3033375400e200229414ccc0c004c4c004cdc019b80008007009153330300121300100815333030300100813371266e05200000a3370000e01229404c94ccc0c4c0b4c0c8dd50008991919191919191919191919299981e99b89337040126eb4c0e0c0fcdd500999b82006375a6074607e6ea804c54ccc0f401454ccc0f401054ccc0f400c54ccc0f400840045280a5014a0294052819baf00b323232323232323374a9000198239824003998239824003198239824002998239ba801033047304800433047304800333047304800233047304800133047375001c609260920026090002608e002608c002608a608a00260880026086002607c6ea809ccdd7808007a99981d19b89375a607e6080608060806080608060786ea809400c5288806a99981c80e0980519b803370000a0060242a6660720362601400a2a666072601400a266e24cdc0a400002666e0000c04852819b890023370666e0801003c0414ccc0dc0644cdc019b8001200101010123370201a0026eb4c0b8c0dcdd500219b8100c001375a6048606a6ea8008c094004c0d8c0ccdd50008b181418191baa01f371266e054ccc0bc04840384cccc08403c0580552006533302f012100b1333302100c0160154801840045281929998190008a51132323300100100322533303500114a0264a66606666e3cdd7181c0010020a511330030030013038001375c606803a6eb0c0ccc0d0c0d0c0d0c0d0c0d0c0d0c0d0c0d0c0d0c0d0c0d0c0c0dd500c99baf0013032303330333033303330333033303330333033302f3754030604e605c6ea806cc098c0b4dd500c1817981818181818181818181818181818161baa015375a604460566ea8050dd6980c98151baa013375a6058605a605a605a605a60526ea8048cdc080100299b81533302500710011333301700200a009480194ccc09401c40104cccc05c0140280252006375a604c60526eacc098c0a4008c0a4004dd5980f98121baa011375a6046604c6eacc08cc098008c098004dd5980e18109baa00c3371e006911003371e008910100375c6032603c6ea8010dd7180b180e9baa003375c602e60386ea800cdd7180a180d9baa002301d301e301e301e301e301e301e301a3754006601e60326ea8008c024004528980d180b9baa00114a26018602c6ea8004c040c054dd50019180a980b180b180b000911119199800800802801111299980a99b884800000454ccc06000852000132323253330183371e6eb8c06400c0244c8c8c94ccc06ccdc79bae301c00300b10011533301e00214800054ccc078c0840084c8c94ccc074cdc79bae301e00200d1001148000dd6980f00098100010b1bad301c002301f002301d00113330060060023370000890009bab3019002301c003301a002162533300e3005300f37540022646464646464646464646464646464646464646464646464a6660526058004264646464649319198008008031129998178008a4c2646600600660660046eb8c0c40054ccc0a4c080c0a8dd5004099191919299981818198010991924c64a66605e604c00226464a666068606e00426493192999819181480089919299981b981d00109924c60520022c607000260686ea800854ccc0c8c0a00044c8c8c8c8c8c94ccc0ecc0f800852616375a607800260780046eb4c0e8004c0e8008dd6981c000981a1baa00216303237540022c606a00260626ea800c54ccc0bcc09400454ccc0c8c0c4dd50018a4c2c2c605e6ea8008c08800c58c0c4004c0c4008c0bc004c0acdd50040b2999814180f98149baa00b132323232533302f3032002149858dd6981800098180011bad302e001302a37540162c603601860340262c6eb0c0a8004c0a8008dd718140009814001181300098130011bad302400130240023022001302200230200013020002375a603c002603c0046eb4c070004c070008dd6980d000980d001180c000980c0011bae30160013016002375c602800260206ea80045894ccc034c010c038dd5000899191919299980a180b8010a4c2c6eb8c054004c054008dd7180980098079baa00116232533300d30040011323253330123015002149858dd7180980098079baa0021533300d30030011323253330123015002149858dd7180980098079baa00216300d37540026e1d2002370e900011807180798078009180698071807180718071807180718071807000980098041baa0042300b001370e90020b1180498050008a4c26cacae6955ceaab9e5573eae815d0aba21",
    };
  },
  { _: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as ILimitOrderBatchWitness;

export interface ILimitOrderLimitOrder {
  new (
    witness: {
      Inline: [
        { VerificationKeyCredential: [string] } | {
          ScriptCredential: [string];
        },
      ];
    } | {
      Pointer: {
        slotNumber: bigint;
        transactionIndex: bigint;
        certificateIndex: bigint;
      };
    },
  ): Validator;
  conf: {
    tag: string;
    beacon: string;
    input: { policy: string; name: string };
    tradableInput: bigint;
    costPerExStep: bigint;
    minMarginalOutput: bigint;
    output: { policy: string; name: string };
    basePrice: { num: bigint; denom: bigint };
    fee: bigint;
    redeemerAddress: {
      paymentCredential: { VerificationKeyCredential: [string] } | {
        ScriptCredential: [string];
      };
      stakeCredential: {
        Inline: [
          { VerificationKeyCredential: [string] } | {
            ScriptCredential: [string];
          },
        ];
      } | {
        Pointer: {
          slotNumber: bigint;
          transactionIndex: bigint;
          certificateIndex: bigint;
        };
      } | null;
    };
    cancellationPkh: string;
    permittedExecutors: Array<string>;
  };
  action: boolean;
}

export const LimitOrderLimitOrder = Object.assign(
  function (
    witness: {
      Inline: [
        { VerificationKeyCredential: [string] } | {
          ScriptCredential: [string];
        },
      ];
    } | {
      Pointer: {
        slotNumber: bigint;
        transactionIndex: bigint;
        certificateIndex: bigint;
      };
    },
  ) {
    return {
      type: "PlutusV2",
      script: applyParamsToScript(
        applyDoubleCborEncoding(
          "5904020100003232323232323222323232232253330093232533300b0041323300100137566022602460246024602460246024601c6ea8008894ccc040004528099299980719baf00d300f301300214a226600600600260260022646464a66601c6014601e6ea80044c94ccc03cc030c040dd5000899191929998090038a99980900108008a5014a066ebcc020c04cdd5001180b180b980b980b980b980b980b980b980b980b98099baa00f3375e600860246ea8c010c048dd5180a98091baa00230043012375400260286eb0c050c054c054c044dd50028b1991191980080080191299980a8008a6103d87a80001323253330143375e6016602c6ea80080144cdd2a40006603000497ae0133004004001301900230170013758600a60206ea8010c04cc040dd50008b180098079baa0052301230130013322323300100100322533301200114a0264a66602066e3cdd7180a8010020a5113300300300130150013758602060226022602260226022602260226022601a6ea8004dd71808180898089808980898089808980898089808980898069baa0093001300c37540044601e00229309b2b19299980598050008a999804180218048008a51153330083005300900114a02c2c6ea8004c8c94ccc01cc010c020dd50028991919191919191919191919191919191919191919191919299981118128010991919191924c646600200200c44a6660500022930991980180198160011bae302a0015333022301f30233754010264646464a666052605800426464931929998141812800899192999816981800109924c64a666056605000226464a66606060660042649318140008b181880098169baa0021533302b3027001132323232323253330343037002149858dd6981a800981a8011bad30330013033002375a6062002605a6ea800858c0acdd50008b181700098151baa0031533302830240011533302b302a37540062930b0b18141baa002302100316302a001302a0023028001302437540102ca666042603c60446ea802c4c8c8c8c94ccc0a0c0ac00852616375a605200260520046eb4c09c004c08cdd50058b180d006180c8098b1bac30230013023002375c60420026042004603e002603e0046eb4c074004c074008c06c004c06c008c064004c064008dd6980b800980b8011bad30150013015002375a60260026026004602200260220046eb8c03c004c03c008dd7180680098049baa0051625333007300430083754002264646464a66601c60220042930b1bae300f001300f002375c601a00260126ea8004588c94ccc01cc0100044c8c94ccc030c03c00852616375c601a00260126ea800854ccc01cc00c0044c8c94ccc030c03c00852616375c601a00260126ea800858c01cdd50009b8748008dc3a4000ae6955ceaab9e5573eae815d0aba201",
        ),
        [witness],
        {
          "dataType": "list",
          "items": [{
            "title": "Referenced",
            "description":
              "Represent a type of object that can be represented either inline (by hash)\n or via a reference (i.e. a pointer to an on-chain location).\n\n This is mainly use for capturing pointers to a stake credential\n registration certificate in the case of so-called pointer addresses.",
            "anyOf": [{
              "title": "Inline",
              "dataType": "constructor",
              "index": 0,
              "fields": [{
                "description":
                  "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                "anyOf": [{
                  "title": "VerificationKeyCredential",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{ "dataType": "bytes" }],
                }, {
                  "title": "ScriptCredential",
                  "dataType": "constructor",
                  "index": 1,
                  "fields": [{ "dataType": "bytes" }],
                }],
              }],
            }, {
              "title": "Pointer",
              "dataType": "constructor",
              "index": 1,
              "fields": [{ "dataType": "integer", "title": "slotNumber" }, {
                "dataType": "integer",
                "title": "transactionIndex",
              }, { "dataType": "integer", "title": "certificateIndex" }],
            }],
          }],
        },
      ),
    };
  },
  {
    conf: {
      "title": "LimitOrderConfig",
      "anyOf": [{
        "title": "LimitOrderConfig",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          { "dataType": "bytes", "title": "tag" },
          { "dataType": "bytes", "title": "beacon" },
          {
            "title": "input",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "integer", "title": "tradableInput" },
          { "dataType": "integer", "title": "costPerExStep" },
          { "dataType": "integer", "title": "minMarginalOutput" },
          {
            "title": "output",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "basePrice",
            "anyOf": [{
              "title": "Rational",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "integer", "title": "num" }, {
                "dataType": "integer",
                "title": "denom",
              }],
            }],
          },
          { "dataType": "integer", "title": "fee" },
          {
            "title": "redeemerAddress",
            "description":
              "A Cardano `Address` typically holding one or two credential references.\n\n Note that legacy bootstrap addresses (a.k.a. 'Byron addresses') are\n completely excluded from Plutus contexts. Thus, from an on-chain\n perspective only exists addresses of type 00, 01, ..., 07 as detailed\n in [CIP-0019 :: Shelley Addresses](https://github.com/cardano-foundation/CIPs/tree/master/CIP-0019/#shelley-addresses).",
            "anyOf": [{
              "title": "Address",
              "dataType": "constructor",
              "index": 0,
              "fields": [{
                "title": "paymentCredential",
                "description":
                  "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                "anyOf": [{
                  "title": "VerificationKeyCredential",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{ "dataType": "bytes" }],
                }, {
                  "title": "ScriptCredential",
                  "dataType": "constructor",
                  "index": 1,
                  "fields": [{ "dataType": "bytes" }],
                }],
              }, {
                "title": "stakeCredential",
                "anyOf": [{
                  "title": "Some",
                  "description": "An optional value.",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{
                    "description":
                      "Represent a type of object that can be represented either inline (by hash)\n or via a reference (i.e. a pointer to an on-chain location).\n\n This is mainly use for capturing pointers to a stake credential\n registration certificate in the case of so-called pointer addresses.",
                    "anyOf": [{
                      "title": "Inline",
                      "dataType": "constructor",
                      "index": 0,
                      "fields": [{
                        "description":
                          "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                        "anyOf": [{
                          "title": "VerificationKeyCredential",
                          "dataType": "constructor",
                          "index": 0,
                          "fields": [{ "dataType": "bytes" }],
                        }, {
                          "title": "ScriptCredential",
                          "dataType": "constructor",
                          "index": 1,
                          "fields": [{ "dataType": "bytes" }],
                        }],
                      }],
                    }, {
                      "title": "Pointer",
                      "dataType": "constructor",
                      "index": 1,
                      "fields": [
                        { "dataType": "integer", "title": "slotNumber" },
                        { "dataType": "integer", "title": "transactionIndex" },
                        { "dataType": "integer", "title": "certificateIndex" },
                      ],
                    }],
                  }],
                }, {
                  "title": "None",
                  "description": "Nothing.",
                  "dataType": "constructor",
                  "index": 1,
                  "fields": [],
                }],
              }],
            }],
          },
          { "dataType": "bytes", "title": "cancellationPkh" },
          {
            "dataType": "list",
            "items": { "dataType": "bytes" },
            "title": "permittedExecutors",
          },
        ],
      }],
    },
  },
  {
    action: {
      "title": "Bool",
      "anyOf": [{
        "title": "False",
        "dataType": "constructor",
        "index": 0,
        "fields": [],
      }, {
        "title": "True",
        "dataType": "constructor",
        "index": 1,
        "fields": [],
      }],
    },
  },
) as unknown as ILimitOrderLimitOrder;

export interface IRoyaltyPoolPoolValidatePool {
  new (): Validator;
  conf: {
    poolnft: { policy: string; name: string };
    poolx: { policy: string; name: string };
    poolY: { policy: string; name: string };
    poolLq: { policy: string; name: string };
    feenum: bigint;
    treasuryFee: bigint;
    royaltyFee: bigint;
    treasuryx: bigint;
    treasuryy: bigint;
    royaltyx: bigint;
    royaltyy: bigint;
    daoPolicy: Array<
      {
        Inline: [
          { VerificationKeyCredential: [string] } | {
            ScriptCredential: [string];
          },
        ];
      } | {
        Pointer: {
          slotNumber: bigint;
          transactionIndex: bigint;
          certificateIndex: bigint;
        };
      }
    >;
    treasuryAddress: string;
    royaltyPubKeyHash_256: string;
    royaltyNonce: bigint;
  };
  action: Data;
}

export const RoyaltyPoolPoolValidatePool = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "590dc9590dc60100003232323232323232323232323232323232323232323232323232323232323232323232322225333023323232323232323232533302c3370e9001001099191929981019809001981880389919191919191919191919191919191919191919299982199b87480100084c8c8c8c8c94cc0e4cdc38058060a99982419b8753330483370e6eb4c128095200014800054ccc120cdc39bad304a0254800852002153330483370e6eb4c128095200414801054ccc120cdc39bad304a025480185200614802120001323232533304b3370e900200109929981e99b873304b2253330450011480004cdc018179bab305230500013002304f0010083304b2253330450011480004cdc018179bab305230500013002304f001007153303d3375e0020122a6607a6605e00a00c2a6607a66e24cdc119b8101101e0153370466e040500540784cdc499b823370202203c02666e08cdc080900980f18268008b182700118238009baa00a153330483370ea66609066e1cdd69825012a4000290000a99982419b87375a609404a90010a40042a66609066e1cdd69825012a4008290020a99982419b87375a609404a90030a400c29004240042646464a66609666e1d200400213253303d3370e6609644a66608a00229000099b80302f375660a460a00026004609e0020106609644a66608a00229000099b80302f375660a460a00026004609e00200e2a6607a66ebc00402454cc0f4cc0bc01401854cc0f4cdc499b823370202203c02a66e08cdc080a00a80f099b893370466e0404407804ccdc119b8101201301e304d00116304e002304700137540142a66609066e1d4ccc120cdc39bad304a0254800052000153330483370e6eb4c128095200214800854ccc120cdc39bad304a0254801052004153330483370e6eb4c128095200614801852008480104c8c8c94cc0f0cdc3998251129998220008a4000266e00c0b8dd59828982780098011827000803998251129998220008a4000266e00c0b8dd598289827800980118270008030a9981e198170020028a9981e19b873370202003a90000a9981e299982599b8848000cdc080980a099b89302d3370466e04044048cdc019b8201402f3370466e0404c050cdc099b810030020013370402466e08cdc080980a19b8133702006004002266e24c0b4cdc119b810130143370066e080480bccdc119b810110123370266e0400c008004cdc100a19b823370202202466e04cdc0801801000899991119191919191919191919191919191919192998281982119ba548000cc160c184044cc160c184040cc160c18403ccc160c184038cc160dd41bad306100d3305837506eb4c184030cc160dd41bad306100b3305837506eb4c18400ccc160dd41bad30610023305837506eb4c184004cc160dd41bad3061306000133058374e6eb0c184018cc160dd49bae30610053305837526eb8c184010cc160dd41bad3061306000405a01415330505330503371266e0810d4ccc17ccdc424000026266e04dd698308009bad30610081337026eb4c184c180004dd69830803a99982f99b884800004c4cdc10099bad306100b1337040246eb4c18402c54cc140cdc4299982f99b884800004c4cdc10099bad306100b1337040246eb4c18402ccdc102199b80533305f337109000009899b81375a60c20026eb4c1840204cdc09bad30613060001375a60c200e90010a9982819b8933704086a6660be66e2120000131337026eb4c18400cdd69830805099b81375a60c20046eb4c1840254ccc17ccdc424000026266e0804cdd69830806099b82012375a60c2018266e214ccc17ccdc424000026266e0804cdd69830806099b82012375a60c201866e0810ccdc0299982f99b884800004c4cdc09bad3061003375a60c2014266e04dd698308011bad30610094800854ccc17ccdc4240000262a660a066e1cdd698308049bad306100213370e6eb4c18401cdd6983098300008a9982819b87375a60c20146eb4c18400c4cdc39bad3061008375a60c200260be00260bc002608260be02260b800260b600260b400260b200260b000260ae00260ac00260aa00260a800260a600260a400260a200260a000260a205c64646464018a66609c66e1d2000002132323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232325333080013370e6e340052038132323232323232324994ccc1f800452616308701003375a002610c020026108020066eb8004c20c04004c2040400c58dd7000984000800983f001999183e11299983b000883d09983c18019840808009801184000800919191919002a9998400099b87480000084c8c8c8c8c8c8c926533307d001149858c218040194ccc21004cdc3a400000426464a66610c0266e1cdc6800a40702646493299983f0008a4c2c610e020062c6eb8004c2180400454ccc21004cdc3a400400426464a66610c0266e1cdc6800a40702646493299983f0008a4c2c610e020062c6eb8004c2180400458c21c04008c20004004dd50009841008008a9998400099b87480080084c8c8c8c8c8c8c8c8c8c926533308001001149858c2240400cdd68009844008009843008019bad001308501001308301003375a0026104020022c61060200460f80026ea8004dd6000983e800983d8019bad001307a0013078003375a00260ee00260ea0066eb4004c1d0004c1c800cdd6800983880098378019bad001306e001306c003375a00260d600260d20066eb4004c1a0004c1980194ccc190cdc3a40000042646464a6660cea660ac66e1c005200013370e002901c0991919299983519b89371a00290200991924ca6660c40022930b18358018b1bae001306a001306800416371a0026eb8004c19800458c19c008c180004dd500098310009830003299982f19b87480000084c8c8c94ccc1854cc140cdc3800a4000266e1c005203813232325333064337126e340052040132324994ccc17000452616306500316375c00260c800260c40082c6e34004dd700098300008b1830801182d0009baa001305c001305a00653330583370e90000010991919299982da9982519b87001480004cdc3800a40702646464a6660bc66e24dc6800a40802646493299982b0008a4c2c60be0062c6eb8004c178004c17001058dc68009bae001305a00116305b0023054001375400260ac00260a800ca6660a466e1d2000002132323253330555330443370e0029000099b87001480e04c8c8c94ccc160cdc49b8d001481004c8c9265333050001149858c16400c58dd7000982c000982b0020b1b8d001375c00260a80022c60aa004609c0026ea8004c14000458c144008c128004dd500419b81013014337020220246eb4c130c0d0c1340a8dd69825981a18260149bad304a3034304b028153330483370ea66609066e1cdd69825012a4000290000a99982419b87375a609404a90010a40042a66609066e1cdd69825012a4008290020a99982419b87375a609404a90030a400c290042400c2646464606266064609a0020046eb0c130c8c130c130c130c130c0c0004c1340a8dd5982598199826000982501209919181819818a60126d8799fd87a9f581cdf06da8cb5e10529efb669eea0c7771d2af94352dadd3dda800af5d6ffff0000137566096606660980026094048609202a60900246eacc11c048dd5982300798228008b1823001181f8009baa0013041304000c3040303f009375a607e6072608000e6eb4c0f8c0e0c0fc03cdd6981e981c181f0029bad303c303b303d004375a6076607460780186eb4c0e8c0ec008dd6981c981d0051981180b002981b000981b981b181a806181a000981a800998181129998150008b0a99981999b8733026323756606c606a606e002606a00200690010981a80089801181a000801181918198081bac30313030303000a375a6060605660620026603401a605e605c00a605c0022c605e00460500026ea8c0acc0a8014c0ac004cc894ccc0a4cdc4001240002c2666050444a66605866e1c00920001302e00113300333702004900118168008010009bad30293028004001375860500026050604e002604e004604c0042930b1980f111299980c800880109980199b8000248008c08c005200023370290000009119baf374e60440046e9cc08800520c09a0c2301d300600123300124a229408cc00800c004888cc068894ccc050004489400454ccc074cdd7980d980f80080209802980f80089801180f0008009191118010019bad301c00123018300200123017300200123016301100122323232323232323374a90001980a9ba83370266e04cc03c004c07801cdd6980f0021bad301e00233015375066e04cdc099807800980f0031bad301e003375a603c603a0046602a6ea0cdc080519807800980f0029980a9ba8533301c53300b323253300f3371e6eb8c080c07c008dd71810180f800899b8f375c60400046eb8c080004c080054c07cc07801c4c8c94cc03ccdc79bae3020301f002375c6040603e002266e3cdd718100011bae30200013020015301f301e0061480004cc03c00405005cdd5980e980e180f003980d800980d000980a180c800980c000980b800980b180c001241fdfffffffffffffffe02466024002004294488ccc04400800400c52811191998020019bae3012001375c602460220026024002444666600800490001199980280124000eb4dd5800801918011ba900122223300d2253330070011005153330103375e601c602400200c2600860286024002260046022002002aae7ccdd2a4000660026ea4008cc004dd4801001aba0489004bd70118031801000918029802800aab9d2323002233002002001230022330020020015734ae895d0918011baa0015573d",
    };
  },
  {
    conf: {
      "title": "Config",
      "anyOf": [{
        "title": "Config",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          {
            "title": "poolnft",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "poolx",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "poolY",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "poolLq",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "integer", "title": "feenum" },
          { "dataType": "integer", "title": "treasuryFee" },
          { "dataType": "integer", "title": "royaltyFee" },
          { "dataType": "integer", "title": "treasuryx" },
          { "dataType": "integer", "title": "treasuryy" },
          { "dataType": "integer", "title": "royaltyx" },
          { "dataType": "integer", "title": "royaltyy" },
          {
            "dataType": "list",
            "items": {
              "title": "Referenced",
              "description":
                "Represent a type of object that can be represented either inline (by hash)\n or via a reference (i.e. a pointer to an on-chain location).\n\n This is mainly use for capturing pointers to a stake credential\n registration certificate in the case of so-called pointer addresses.",
              "anyOf": [{
                "title": "Inline",
                "dataType": "constructor",
                "index": 0,
                "fields": [{
                  "description":
                    "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                  "anyOf": [{
                    "title": "VerificationKeyCredential",
                    "dataType": "constructor",
                    "index": 0,
                    "fields": [{ "dataType": "bytes" }],
                  }, {
                    "title": "ScriptCredential",
                    "dataType": "constructor",
                    "index": 1,
                    "fields": [{ "dataType": "bytes" }],
                  }],
                }],
              }, {
                "title": "Pointer",
                "dataType": "constructor",
                "index": 1,
                "fields": [{ "dataType": "integer", "title": "slotNumber" }, {
                  "dataType": "integer",
                  "title": "transactionIndex",
                }, { "dataType": "integer", "title": "certificateIndex" }],
              }],
            },
            "title": "daoPolicy",
          },
          { "dataType": "bytes", "title": "treasuryAddress" },
          { "dataType": "bytes", "title": "royaltyPubKeyHash_256" },
          { "dataType": "integer", "title": "royaltyNonce" },
        ],
      }],
    },
  },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolPoolValidatePool;

export interface IRoyaltyPoolWithdrawRoyaltyRequestValidate {
  new (): Validator;
  conf: {
    withdrawdata: {
      poolnft: { policy: string; name: string };
      withdrawroyaltyx: bigint;
      withdrawroyaltyy: bigint;
      royaltyaddress: string;
      royaltypubkey: string;
      exfee: bigint;
    };
    signature: string;
  };
  action: Data;
}

export const RoyaltyPoolWithdrawRoyaltyRequestValidate = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "5907d05907cd010000323232323232323232323232323232323232323232323222253330163232323232323232323232323232323253330263370ea66604c66e1cdd698141813802a4000290000a40049000099191919191919299981699b87480080084c8c8c8c8c94cc0814cc080cdc49bad30340193302400130340031337126eb4c0d0060cc090004c0d0c0cc00c54cc080cdc39998189112999818000880109980199b8000248008c0d800520000104801054cc080cdc399b81009533303232325330223371e6eb8c0d8c0d4008dd7181b181a800899b8f375c606c0046eb8c0d8004c0d80a8c0d4c0d000c4cdc099812000981a0019bad30340191533303232325330223371e6eb8c0d8c0d4008dd7181b181a800899b8f375c606c0046eb8c0d8004c0d80a8c0d4c0d0c0cc00c4cdc099812000981a18198019bad3034018133024001029375a6068606602c2a66040664466ebcdd3981b8011ba730370013034006303400b13375e606800860680346644646464a66606c646464a66607266e1d20020021323232533303c3370e90000010a5013375e6e9c010dd3800981f80118180009baa0071323232533303c3370e90010010a5013375e6e9c010dd3800981f80118180009baa007303c002302d001375400220042c6464646464a66607466e1d2000002132323232533303e3370e9000001099191919299982119b8748008008584cdd2a4000002608a004606c0026ea8004c10000458c104008c0c8004dd5000981e000899ba5480080d0c0f4008c0b8004dd5000981c181b981c800981b981c0029bab32323253330373370e90010010b0a99981b99b8f375c607200200c260726070607400e2c607400460560026ea8004c8c0d8c0dc004c0d4c0d800cdd7181980b19ba5480080accc0b4dd698190078069818000981880099191919299981899b87480100084c8c8c8c8c80154ccc0d4cdc3a400000426464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464a6660ce66e1cdc6800a4070264646464646464649329998348008a4c2c60dc0066eb4004c1b4004c1ac00cdd7000983500098340018b1bae001306700130650033323063225333061001106013305e300330680013002306700123232323200553330673370e900000109919191919191924ca6660d00022930b1836803299983599b87480000084c8c94ccc1b4cdc39b8d001480e04c8c9265333069001149858c1b800c58dd700098368008a99983599b87480080084c8c94ccc1b4cdc39b8d001480e04c8c9265333069001149858c1b800c58dd700098368008b1837001182f8009baa0013069001153330673370e900100109919191919191919191924ca6660d60022930b18380019bad001306f001306d003375a00260d800260d40066eb4004c1a400458c1a8008c16c004dd50009bac00130640013062003375a00260c200260be0066eb4004c178004c17000cdd6800982d800982c8019bad00130580013056003375a00260aa00260a60066eb4004c148004c14000cdd680098278009826803299982599b87480000084c8c8c94ccc1394cc0f8cdc3800a4000266e1c005203813232325333051337126e340052040132324994ccc13400452616305200316375c00260a2002609e0082c6e34004dd700098268008b1827001181f8009baa0013049001304700653330453370e90000010991919299982429981c19b87001480004cdc3800a40702646464a66609666e24dc6800a4080264649329998238008a4c2c60980062c6eb8004c12c004c12401058dc68009bae0013047001163048002303900137540026086002608200ca66607e66e1d2000002132323253330425330323370e0029000099b87001480e04c8c8c94ccc114cdc49b8d001481004c8c9265333041001149858c11800c58dd7000982280098218020b1b8d001375c00260820022c608400460660026ea8004c0f4004c0ec0194ccc0e4cdc3a40000042646464a666078a6605866e1c005200013370e002901c0991919299981f99b89371a00290200991924ca6660760022930b18200018b1bae001303f001303d00416371a0026eb8004c0ec00458c0f0008c0b4004dd5000981b8008b181c00118148009baa001303300116303400230250013754002605e60526060605e605c0102c606000460420026ea8c0b0c0ac050cc06c004080dd599181598151816000981518148009815000998119bad302800600413322332302822533302600114a02a66605666ebcc0b400400c5288980118160009ba900100237586050646050605060506046002604e0106eb8c0a002cc0a0004cc084dd698130028011bac30250053758604800a604400260420026044016603e603e0026040603e00e603a00260380026036002603400260366034002603400860320022930b1119980a0010008018a5023301100100214a244646660080066eb8c04c004dd71809980900098098009111999802001240004666600a00490003ad3756002006460046ea40048888cc038894ccc030004401454ccc044cdd79803980980080309802180a9809800898011809000800aab9d3374a9000198009ba9002330013752004006ae812201004bd702ab9f2300630060012253330053371000490000b0998018010009800911299980299b87002480004c01c0044cc00ccdc080124004600c002464600446600400400246004466004004002ae695d12ba1230023754002aae781",
    };
  },
  {
    conf: {
      "title": "Config",
      "anyOf": [{
        "title": "Config",
        "dataType": "constructor",
        "index": 0,
        "fields": [{
          "title": "withdrawdata",
          "anyOf": [{
            "title": "Config",
            "dataType": "constructor",
            "index": 0,
            "fields": [
              {
                "title": "poolnft",
                "anyOf": [{
                  "title": "Asset",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{ "dataType": "bytes", "title": "policy" }, {
                    "dataType": "bytes",
                    "title": "name",
                  }],
                }],
              },
              { "dataType": "integer", "title": "withdrawroyaltyx" },
              { "dataType": "integer", "title": "withdrawroyaltyy" },
              { "dataType": "bytes", "title": "royaltyaddress" },
              { "dataType": "bytes", "title": "royaltypubkey" },
              { "dataType": "integer", "title": "exfee" },
            ],
          }],
        }, { "dataType": "bytes", "title": "signature" }],
      }],
    },
  },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolWithdrawRoyaltyRequestValidate;

export interface IRoyaltyPoolWithdrawRoyaltyRequestDummyValidate {
  new (): Validator;
  conf: {
    withdrawdata: {
      poolnft: { policy: string; name: string };
      withdrawroyaltyx: bigint;
      withdrawroyaltyy: bigint;
      royaltyaddress: string;
      royaltypubkey: string;
      exfee: bigint;
    };
    poolnonce: bigint;
  };
  action: Data;
}

export const RoyaltyPoolWithdrawRoyaltyRequestDummyValidate = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "5907d05907cd010000323232323232323232323232323232323232323232323222253330163232323232323232323232323232323253330263370ea66604c66e1cdd698141813802a4000290000a40049000099191919191919299981699b87480080084c8c8c8c8c94cc0814cc080cdc49bad30340193302400130340031337126eb4c0d0060cc090004c0d0c0cc00c54cc080cdc39998189112999818000880109980199b8000248008c0d800520000104801054cc080cdc399b81009533303232325330223371e6eb8c0d8c0d4008dd7181b181a800899b8f375c606c0046eb8c0d8004c0d80a8c0d4c0d000c4cdc099812000981a0019bad30340191533303232325330223371e6eb8c0d8c0d4008dd7181b181a800899b8f375c606c0046eb8c0d8004c0d80a8c0d4c0d0c0cc00c4cdc099812000981a18198019bad3034018133024001029375a6068606602c2a66040664466ebcdd3981b8011ba730370013034006303400b13375e606800860680346644646464a66606c646464a66607266e1d20020021323232533303c3370e90000010a5013375e6e9c010dd3800981f80118180009baa0071323232533303c3370e90010010a5013375e6e9c010dd3800981f80118180009baa007303c002302d001375400220042c6464646464a66607466e1d2000002132323232533303e3370e9000001099191919299982119b8748008008584cdd2a4000002608a004606c0026ea8004c10000458c104008c0c8004dd5000981e000899ba5480080d0c0f4008c0b8004dd5000981c181b981c800981b981c0029bab32323253330373370e90010010b0a99981b99b8f375c607200200c260726070607400e2c607400460560026ea8004c8c0d8c0dc004c0d4c0d800cdd7181980b19ba5480080accc0b4dd698190078069818000981880099191919299981899b87480100084c8c8c8c8c80154ccc0d4cdc3a400000426464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464a6660ce66e1cdc6800a4070264646464646464649329998348008a4c2c60dc0066eb4004c1b4004c1ac00cdd7000983500098340018b1bae001306700130650033323063225333061001106013305e300330680013002306700123232323200553330673370e900000109919191919191924ca6660d00022930b1836803299983599b87480000084c8c94ccc1b4cdc39b8d001480e04c8c9265333069001149858c1b800c58dd700098368008a99983599b87480080084c8c94ccc1b4cdc39b8d001480e04c8c9265333069001149858c1b800c58dd700098368008b1837001182f8009baa0013069001153330673370e900100109919191919191919191924ca6660d60022930b18380019bad001306f001306d003375a00260d800260d40066eb4004c1a400458c1a8008c16c004dd50009bac00130640013062003375a00260c200260be0066eb4004c178004c17000cdd6800982d800982c8019bad00130580013056003375a00260aa00260a60066eb4004c148004c14000cdd680098278009826803299982599b87480000084c8c8c94ccc1394cc0f8cdc3800a4000266e1c005203813232325333051337126e340052040132324994ccc13400452616305200316375c00260a2002609e0082c6e34004dd700098268008b1827001181f8009baa0013049001304700653330453370e90000010991919299982429981c19b87001480004cdc3800a40702646464a66609666e24dc6800a4080264649329998238008a4c2c60980062c6eb8004c12c004c12401058dc68009bae0013047001163048002303900137540026086002608200ca66607e66e1d2000002132323253330425330323370e0029000099b87001480e04c8c8c94ccc114cdc49b8d001481004c8c9265333041001149858c11800c58dd7000982280098218020b1b8d001375c00260820022c608400460660026ea8004c0f4004c0ec0194ccc0e4cdc3a40000042646464a666078a6605866e1c005200013370e002901c0991919299981f99b89371a00290200991924ca6660760022930b18200018b1bae001303f001303d00416371a0026eb8004c0ec00458c0f0008c0b4004dd5000981b8008b181c00118148009baa001303300116303400230250013754002605e60526060605e605c0102c606000460420026ea8c0b0c0ac050cc06c004080dd599181598151816000981518148009815000998119bad302800600413322332302822533302600114a02a66605666ebcc0b400400c5288980118160009ba900100237586050646050605060506046002604e0106eb8c0a002cc0a0004cc084dd698130028011bac30250053758604800a604400260420026044016603e603e0026040603e00e603a00260380026036002603400260366034002603400860320022930b1119980a0010008018a5023301100100214a244646660080066eb8c04c004dd71809980900098098009111999802001240004666600a00490003ad3756002006460046ea40048888cc038894ccc030004401454ccc044cdd79803980980080309802180a9809800898011809000800aab9d3374a9000198009ba9002330013752004006ae812201004bd702ab9f2300630060012253330053371000490000b0998018010009800911299980299b87002480004c01c0044cc00ccdc080124004600c002464600446600400400246004466004004002ae695d12ba1230023754002aae781",
    };
  },
  {
    conf: {
      "title": "Config",
      "anyOf": [{
        "title": "Config",
        "dataType": "constructor",
        "index": 0,
        "fields": [{
          "title": "withdrawdata",
          "anyOf": [{
            "title": "Config",
            "dataType": "constructor",
            "index": 0,
            "fields": [
              {
                "title": "poolnft",
                "anyOf": [{
                  "title": "Asset",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{ "dataType": "bytes", "title": "policy" }, {
                    "dataType": "bytes",
                    "title": "name",
                  }],
                }],
              },
              { "dataType": "integer", "title": "withdrawroyaltyx" },
              { "dataType": "integer", "title": "withdrawroyaltyy" },
              { "dataType": "bytes", "title": "royaltyaddress" },
              { "dataType": "bytes", "title": "royaltypubkey" },
              { "dataType": "integer", "title": "exfee" },
            ],
          }],
        }, { "dataType": "integer", "title": "poolnonce" }],
      }],
    },
  },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolWithdrawRoyaltyRequestDummyValidate;

export interface IRoyaltyPoolDaoV1RequestValidate {
  new (): Validator;
  conf: {
    daoaction: bigint;
    poolnft: { policy: string; name: string };
    poolfee: bigint;
    treasuryfee: bigint;
    adminaddress: Array<
      {
        Inline: [
          { VerificationKeyCredential: [string] } | {
            ScriptCredential: [string];
          },
        ];
      } | {
        Pointer: {
          slotNumber: bigint;
          transactionIndex: bigint;
          certificateIndex: bigint;
        };
      }
    >;
    pooladdress: {
      paymentCredential: { VerificationKeyCredential: [string] } | {
        ScriptCredential: [string];
      };
      stakeCredential: {
        Inline: [
          { VerificationKeyCredential: [string] } | {
            ScriptCredential: [string];
          },
        ];
      } | {
        Pointer: {
          slotNumber: bigint;
          transactionIndex: bigint;
          certificateIndex: bigint;
        };
      } | null;
    };
    treasuryaddress: string;
    treasuryxwithdraw: bigint;
    treasuryywithdraw: bigint;
    requestorpkh: string;
    signatures: Array<string>;
    exfee: bigint;
  };
  action: Data;
}

export const RoyaltyPoolDaoV1RequestValidate = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "59040b59040801000032323232323232323232323232323232323232323232323222253330133232323232323232323232533301e3370ea66603c66e1cdd6980f9812000a4000290000a40049000099191919191919191919191919299981599b87480080084c8c94cc078cdc399810000981700c240042a6603c66e1cccc0ac8894ccc0bc00440084cc00ccdc000124004606c0029000007a40082a6603c66ebcc0b800cc0b802854cc078c8cc0049288a50332332223302f2253330320011225001153330333375e605460680020082600a60680022600460720020024644460040066eb4c0dc00400530126d8799fd87a9f581c66e711a4bf9ddf46ff239143870b6893055a4fd4dea9f99fed6665cdffff0000213370e6604000c04a66e04020dd698171819181980a9bab32302e30333034001302d303200b375664605a646066605e002606600260580302c606400460400026ea8c0a4c0b8054cc88c8c8c94ccc0b0c8c8c94ccc0bccdc3a40040042646464a66606466e1d200000214a0266ebcdd38021ba700130390023027001375400e2646464a66606466e1d200200214a0266ebcdd38021ba700130390023027001375400e606c00460480026ea8004400858c8c8c8c8c94ccc0c0cdc3a4000004264646464a66606866e1d200000213232323253330383370e90010010b099ba548000004c0fc008c0b4004dd5000981a8008b181d80118148009baa001303100113374a9001015981b80118128009baa001302d30323033001302c30320053756646464a66605a66e1d2002002161533302d3371e6eb8c0b80040184c0b8c0ccc0d001c58c0d0008c088004dd50009918159818800981518180019bae302800f3374a9001011198111bad30270090073301800101d375664604c60566058002604a605400260540026603c6eb4c08c018010c0a0004cc070dd698108028011bac30200053758603e00a26644664603e44a666044002294054ccc08ccdd798120008018a5113002302900137520020046eb0c07cc080c090010dd7180f803181180098110009811005181018100009810180d003180f000980e800980c180e000980d800980d802180d0008a4c2c44666022004002006294088c8ccc01000cdd718088009bae3011301600130160012223333004002480008cccc014009200075a6eac00400c8c008dd480091111980611299980780088028a99980819baf3007301100100613004301830110011300230160010015573a66e952000330013752004660026ea400800d5d0245004bd7011299980319b8800248000584cc00c008004c0048894ccc018cdc3801240002600e00226600666e040092002300c0012323002233002002001230022330020020015573eae695d0918029801000918021801000918019801000918011801000aba2230023754002aae781",
    };
  },
  {
    conf: {
      "title": "Config",
      "anyOf": [{
        "title": "Config",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          { "dataType": "integer", "title": "daoaction" },
          {
            "title": "poolnft",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "integer", "title": "poolfee" },
          { "dataType": "integer", "title": "treasuryfee" },
          {
            "dataType": "list",
            "items": {
              "title": "Referenced",
              "description":
                "Represent a type of object that can be represented either inline (by hash)\n or via a reference (i.e. a pointer to an on-chain location).\n\n This is mainly use for capturing pointers to a stake credential\n registration certificate in the case of so-called pointer addresses.",
              "anyOf": [{
                "title": "Inline",
                "dataType": "constructor",
                "index": 0,
                "fields": [{
                  "description":
                    "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                  "anyOf": [{
                    "title": "VerificationKeyCredential",
                    "dataType": "constructor",
                    "index": 0,
                    "fields": [{ "dataType": "bytes" }],
                  }, {
                    "title": "ScriptCredential",
                    "dataType": "constructor",
                    "index": 1,
                    "fields": [{ "dataType": "bytes" }],
                  }],
                }],
              }, {
                "title": "Pointer",
                "dataType": "constructor",
                "index": 1,
                "fields": [{ "dataType": "integer", "title": "slotNumber" }, {
                  "dataType": "integer",
                  "title": "transactionIndex",
                }, { "dataType": "integer", "title": "certificateIndex" }],
              }],
            },
            "title": "adminaddress",
          },
          {
            "title": "pooladdress",
            "description":
              "A Cardano `Address` typically holding one or two credential references.\n\n Note that legacy bootstrap addresses (a.k.a. 'Byron addresses') are\n completely excluded from Plutus contexts. Thus, from an on-chain\n perspective only exists addresses of type 00, 01, ..., 07 as detailed\n in [CIP-0019 :: Shelley Addresses](https://github.com/cardano-foundation/CIPs/tree/master/CIP-0019/#shelley-addresses).",
            "anyOf": [{
              "title": "Address",
              "dataType": "constructor",
              "index": 0,
              "fields": [{
                "title": "paymentCredential",
                "description":
                  "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                "anyOf": [{
                  "title": "VerificationKeyCredential",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{ "dataType": "bytes" }],
                }, {
                  "title": "ScriptCredential",
                  "dataType": "constructor",
                  "index": 1,
                  "fields": [{ "dataType": "bytes" }],
                }],
              }, {
                "title": "stakeCredential",
                "anyOf": [{
                  "title": "Some",
                  "description": "An optional value.",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{
                    "description":
                      "Represent a type of object that can be represented either inline (by hash)\n or via a reference (i.e. a pointer to an on-chain location).\n\n This is mainly use for capturing pointers to a stake credential\n registration certificate in the case of so-called pointer addresses.",
                    "anyOf": [{
                      "title": "Inline",
                      "dataType": "constructor",
                      "index": 0,
                      "fields": [{
                        "description":
                          "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                        "anyOf": [{
                          "title": "VerificationKeyCredential",
                          "dataType": "constructor",
                          "index": 0,
                          "fields": [{ "dataType": "bytes" }],
                        }, {
                          "title": "ScriptCredential",
                          "dataType": "constructor",
                          "index": 1,
                          "fields": [{ "dataType": "bytes" }],
                        }],
                      }],
                    }, {
                      "title": "Pointer",
                      "dataType": "constructor",
                      "index": 1,
                      "fields": [
                        { "dataType": "integer", "title": "slotNumber" },
                        { "dataType": "integer", "title": "transactionIndex" },
                        { "dataType": "integer", "title": "certificateIndex" },
                      ],
                    }],
                  }],
                }, {
                  "title": "None",
                  "description": "Nothing.",
                  "dataType": "constructor",
                  "index": 1,
                  "fields": [],
                }],
              }],
            }],
          },
          { "dataType": "bytes", "title": "treasuryaddress" },
          { "dataType": "integer", "title": "treasuryxwithdraw" },
          { "dataType": "integer", "title": "treasuryywithdraw" },
          { "dataType": "bytes", "title": "requestorpkh" },
          {
            "dataType": "list",
            "items": { "dataType": "bytes" },
            "title": "signatures",
          },
          { "dataType": "integer", "title": "exfee" },
        ],
      }],
    },
  },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolDaoV1RequestValidate;

export interface IRoyaltyPoolDaoV1Validate {
  new (): Validator;
  conf: Data;
  action: Data;
}

export const RoyaltyPoolDaoV1Validate = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "59143259142f01000032323232323232323232323232323232323232323232323232323232323222533301b323232323232323253330233370e9002001099191919191919191919191919191919191919191919191919191919191919191919191919191919191929982099b87303e0294801054cc1054ccc12008852889991199827801000a503574660a004466ebcc138c13c088dd481e8a9982099999999999111111111299825a99825991929982699b8f375c60b660b80046eb8c16cc1700044cdc79bae305b002375c60b600260b860b405260b660b20302a6609666ebcc16405cc16409c54cc12ccdd7982c80b182c8130a9982599baf30590153059025153304b3375e60b201e60b203e2a6609666ebcc164038c16407854cc12ccdd7982c809182c8110a9982599baf305900b305901b13370e66e00dd6982c982d005a40046eb4c164c16806c4cdc48039999182b911299982b000880109980199802001182e800982f00091299982c800899b800024800840092000333230572225333056002104315333056001104313305333004305d002305d00133003305e002305e0012233372a6eb8c178008004dd7182d0011999182b911299982b00108220a99982b000882209982999802182e801182e80099801982f001182f0009119bb0002375200201201466460ae44a6660aa002297ac41330523003305c0013002305d0012337146eb8004dd999ba548000cc1414ccc160cdc38012400026ea12000153330583370e004900109ba84800854ccc160cdc38012400826ea12004153330583370e004900309ba84801854ccc160cdc38012401026ea1200813750900519828182d014998281ba800733050375000c660a06e9c014cc140010cc140dd4801998281ba8337026eb4c168088dd6982d009198281ba8337026eb4c168084dd6982d008998281ba8375a60b460b601809e01097ac51200bb1d2db22f9b641f0afe8d8a398279cb778d8f86167500f7e63ebbdc35b4d690081204e8221615500dbf6737b02992610ffeed82da6826dc3d9729febf1d32d766615008120ae536160ccec4f078982396125773d509072397e35ed6fab7af2a762ca14731800812004e3a257bcb0306c27e796bc16d1b7bde8f2306dc1d6aa344f6043ef48bd7fd800812083d3aa4ccd1c72ff7a27c032394f44b699778b6050290e37fd82bc295f5caf18008120a7d30e99673c57638bdb65b5a0554ddee3135131940a41bbd3534b0d4c709506001bac304f02f3758609e60a005e90041bad304f01a375a609e0326eb0c13c04c03cdd71827809299982699b87375a609e06490000a40002a66609a66e1cdd6982781924004290010a99982699b87375a609e06490020a40082a66609a66e1cdd698278192400c290030a99982699b87375a609e06490040a4010290050a99982699b87533304d3370e6eb4c13c0c9200014800054ccc134cdc39bad304f03248008520021533304d3370e6eb4c13c0c9200414801054ccc134cdc39bad304f03248018520061533304d3370e6eb4c13c0c920081480205200a4800054cc104cdc39bad304f009375a609e0322a6608266ebcdd39bac304f003374e6eb0c13c04c54cc104cdc79bae304f002375c609e0242a660826608002001e2a66082646464a66088a6608866e1ccc0ec0a0c148038cc0ec088c14803854cc110cdc399b813303b02230520103303b0283052010337026eb4c148068dd69829005099b87337026607604460a401e6607605060a401e66e04dd6982900c9bad305200915330445330443370e6607600260a4020607466e04dd6982900d1bad305200a13370e6607600260a401e607466e04dd6982900c9bad305200915330443371e6eb8c148014dd7182900a8a9982219b873303b0280034800854cc110cdc4a40006eb4c14806854cc110cdc4a40006eb4c14806454cc110cdc3a999828299824991929982319b8f375c60a860aa0046eb8c150c1540044cdc79bae3054002375c60a800260aa08060a860a402026464a6608c66e3cdd7182a182a8011bae3054305500113371e6eb8c150008dd7182a000982a820182a18290078a400026607605007ea6660a0a660926464a6608c66e3cdd7182a182a8011bae3054305500113371e6eb8c150008dd7182a000982a820182a18290080991929982319b8f375c60a860aa0046eb8c150c1540044cdc79bae3054002375c60a800260aa08060a860a401e2900009981d81101f899b873304f22533304d0011480004cdc018219bab30573054001300230550010283304f22533304d0011480004cdc018219bab3057305400130023055001022375660a260a460a6002664609c44a6660980022c264646464a6660aa66e1d2000002130063058005153330553371e6eb8c15c00401c4c15c0144c018c160014c164008c150004dd5182a182b00099182a182b00098298009bae3050003029304f01e13370e6eb4c13c028dd6982780d0a99982699b87533304d3370e6eb4c13c0c9200014800054ccc134cdc39bad304f03248008520021533304d3370e6eb4c13c0c9200414801054ccc134cdc39bad304f03248018520061533304d3370e6eb4c13c0c920081480205200a480084c8c94cc10ccdc39bad305100b375a60a20362a66086a6608666ebcc144024c1440644cdd79828804182880c0a9982199baf374e6eb0c144014dd39bac305101515330433371e6eb8c144010dd7182880a0a99821998208138108a99821991919299982919b87480080084c8c8c94ccc154cdc3a400000429404cdd79ba7004374e00260b200460a80026ea80104c8c8c94ccc154cdc3a400400429404cdd79ba7004374e00260b200460a80026ea8010c158008c144004dd5001099b87375a60a20186eb4c144070c140c148040c13cc14404054ccc134cdc3a99982699b87375a609e06490000a40002a66609a66e1cdd6982781924004290010a99982699b87375a609e06490020a40082a66609a66e1cdd698278192400c290030a99982699b87375a609e06490040a401029005240082a66082a6608266ebcc13c01cc13c05c4cdd79827803182780b0a9982099baf374e6eb0c13c00cdd39bac304f01315330413371e6eb8c13c008dd718278090a99820a998209981f81280f8998200080078a9982099b87375a609e0146eb4c13c06854cc104cdc49bad304f01948302e0084cdc4a40046eb4c13c06454ccc134cdc3a99982699b87375a609e06490000a40002a66609a66e1cdd6982781924004290010a99982699b87375a609e06490020a40082a66609a66e1cdd698278192400c290030a99982699b87375a609e06490040a4010290052400c2a6608266e1cdd698278049bad304f01915330415330413375e609e00e609e02e266ebcc13c018c13c05854cc104cdd79ba73758609e0066e9cdd618278098a99820a998209981f81280f899820008007899b87375a609e0146eb4c13c06854ccc134cdc3a99982699b87375a609e06490000a40002a66609a66e1cdd6982781924004290010a99982699b87375a609e06490020a40082a66609a66e1cdd698278192400c290030a99982699b87375a609e06490040a401029005240102a6608266e1cdd698278049bad304f01915330415330413375e609e00e609e02e266ebcc13c018c13c05854cc104cdc79bae304f002375c609e0242a66082a660826607e04a03e26608002001e266e1cdd698278051bad304f01a15330414a22a6608266e1cdd698278049bad304f01915330415330413375e609e00e609e02e266ebcc13c018c13c05854cc104cdc79bae304f002375c609e0242a6608266ebcdd39bac304f003374e6eb0c13c04c54cc1054cc104cc0fc09407c4cc10004003c54cc104cdc49bad304f01a482fa68304cdc4a40046eb4c13c068c13c004c138004c134004c130004c12c004c128004c124004c120004c11c004c118004c114004c110004c10c004c10c058c100c108048c8c100c108004c0fcc100058c0fc004c0f8004c0f4004c0f0004c0ec004c0e8004c0e4004c0e0004c0dc004c0d8004c0d4004c0d0004c0cc004c0cc008dd598181818981900119191919299981899b87480100084c8c8c8c8c80154ccc0d4cdc3a400000426464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464a6660ce66e1cdc6800a4070264646464646464649329998348008a4c2c60e00066eb4004c1b4004c1b400cdd7000983500098350018b1bae001306700130670033305923232323200553330673370e900000109919191919191924ca6660d00022930b1837803299983599b87480000084c8c94ccc1b4cdc39b8d001480e04c8c9265333069001149858c1c000c58dd700098368008a99983599b87480080084c8c94ccc1b4cdc39b8d001480e04c8c9265333069001149858c1c000c58dd700098368008b183780118350009baa0013069001153330673370e900100109919191919191919191924ca6660d60022930b18390019bad001306f001306f003375a00260d800260d80066eb4004c1a400458c1ac008c198004dd50009bac00130640013064003375a00260c200260c20066eb4004c178004c17800cdd6800982d800982d8019bad00130580013058003375a00260aa00260aa0066eb4004c148004c14800cdd680098278009827803299982599b87480000084c8c8c94ccc1394cc11ccdc3800a4000266e1c005203813232325333051337126e340052040132324994ccc13400452616305400316375c00260a200260a20082c6e34004dd700098268008b182780118250009baa0013049001304900653330453370e90000010991919299982429982099b87001480004cdc3800a40702646464a66609666e24dc6800a4080264649329998238008a4c2c609c0062c6eb8004c12c004c12c01058dc68009bae0013047001163049002304400137540026086002608600ca66607e66e1d20000021323232533304253303b3370e0029000099b87001480e04c8c8c94ccc114cdc49b8d001481004c8c9265333041001149858c12000c58dd7000982280098228020b1b8d001375c00260820022c6086004607c0026ea8004c0f4004c0f40194ccc0e4cdc3a40000042646464a666078a6606a66e1c005200013370e002901c0991919299981f99b89371a00290200991924ca6660760022930b18210018b1bae001303f001303f00416371a0026eb8004c0ec00458c0f4008c0e0004dd5000981b8008b181c801181a0009baa001303300116303500230300013754002605e60526062002660526eb4c0b803c01cdd5991817181798180009816981718178009981399b8148008dd698160070031919191919299981719b87480100084c8c8c8c8c80154ccc0c8cdc3a400000426464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464a6660c866e1cdc6800a4070264646464646464649329998330008a4c2c60da0066eb4004c1a8004c1a800cdd7000983380098338018b1bae001306400130640033305623232323200553330643370e900000109919191919191924ca6660ca0022930b1836003299983419b87480000084c8c94ccc1a8cdc39b8d001480e04c8c9265333066001149858c1b400c58dd700098350008a99983419b87480080084c8c94ccc1a8cdc39b8d001480e04c8c9265333066001149858c1b400c58dd700098350008b183600118338009baa0013066001153330643370e900100109919191919191919191924ca6660d00022930b18378019bad001306c001306c003375a00260d200260d20066eb4004c19800458c1a0008c18c004dd50009bac00130610013061003375a00260bc00260bc0066eb4004c16c004c16c00cdd6800982c000982c0019bad00130550013055003375a00260a400260a40066eb4004c13c004c13c00cdd680098260009826003299982419b87480000084c8c8c94ccc12d4cc110cdc3800a4000266e1c00520381323232533304e337126e340052040132324994ccc12800452616305100316375c002609c002609c0082c6e34004dd700098250008b182600118238009baa0013046001304600653330423370e900000109919192999822a9981f19b87001480004cdc3800a40702646464a66609066e24dc6800a4080264649329998220008a4c2c60960062c6eb8004c120004c12001058dc68009bae0013044001163046002304100137540026080002608000ca66607866e1d20000021323232533303f5330383370e0029000099b87001480e04c8c8c94ccc108cdc49b8d001481004c8c926533303e001149858c11400c58dd7000982100098210020b1b8d001375c002607c0022c608000460760026ea8004c0e8004c0e80194ccc0d8cdc3a40000042646464a666072a6606466e1c005200013370e002901c0991919299981e19b89371a00290200991924ca6660700022930b181f8018b1bae001303c001303c00416371a0026eb8004c0e000458c0e8008c0d4004dd5000981a0008b181b00118188009baa0013030001163032002302d00137540026058604c605c002605660580046eacc8c0acc0b0c0b4004c0a8c0ac004c0ac004cc08cdd698140050011bac3027302830280023758604c002604e604a0082c604e00460440026ea8c088c08c004c08c014c084004c080004c07c004c07c008526164bd6825eb7bdb1808cdc0a400000244646660080066eb8c068004dd7180d180d800980d8009111999802001240004666600a00490003ad3756002006460046ea40048888cc054894ccc04c004401454ccc060cdd7980c980d00080309802180e980d00089801180d80080099ba548000cc024dd4800998049ba9001008489003300e222533300d0011002133003337000049001180a800a40004466ebcdd30011ba6001223375e6e9cc04c008dd39809800911998070010008018a502300a22533300800110041330053003300f001300230100014bd702ba023300800100214a2aae7c8c020c020004894ccc014cdc4001240002c2660060040026002444a66600a66e1c0092000130070011330033370200490011804000919180111980100100091801119801001000ab9a5573aae855d1118011baa0015573c1",
    };
  },
  { conf: { "title": "Data", "description": "Any Plutus data." } },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolDaoV1Validate;

export interface IRoyaltyPoolDaoV1DummyValidate {
  new (): Validator;
  conf: {
    daoaction: bigint;
    poolnft: { policy: string; name: string };
    poolfee: bigint;
    treasuryfee: bigint;
    adminaddress: Array<
      {
        Inline: [
          { VerificationKeyCredential: [string] } | {
            ScriptCredential: [string];
          },
        ];
      } | {
        Pointer: {
          slotNumber: bigint;
          transactionIndex: bigint;
          certificateIndex: bigint;
        };
      }
    >;
    pooladdress: {
      paymentCredential: { VerificationKeyCredential: [string] } | {
        ScriptCredential: [string];
      };
      stakeCredential: {
        Inline: [
          { VerificationKeyCredential: [string] } | {
            ScriptCredential: [string];
          },
        ];
      } | {
        Pointer: {
          slotNumber: bigint;
          transactionIndex: bigint;
          certificateIndex: bigint;
        };
      } | null;
    };
    treasuryaddress: string;
    treasuryxwithdraw: bigint;
    treasuryywithdraw: bigint;
    nonce: bigint;
  };
  action: Data;
}

export const RoyaltyPoolDaoV1DummyValidate = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "59120a591207010000323232323232323232323232323232323232323232323232323222533301732323232323232533301e3370e9002001099191919191919191919191919191919191919191919191919191919191919191919191919191919191929981c19b87333047222533304600110021330033370000490011827000a400005290020a9981c29998218110a511332233304a0020014a06ae8cc12c088cdd7982498250111ba9488100153303833333333322222222253304153304132325330433371e6eb8c154c158008dd7182a982b000899b8f375c60aa0046eb8c154004c158c150060c154c14c09454cc104cdd79829812182980b0a9982099baf3053023305301515330413375e60a604460a60282a6608266ebcc14c070c14c03854cc104cdd7982980d98298068a9982099baf305301f305301115330413375e60a603060a6014266e1ccdc01bad3053305401848008dd69829982a005099b89007333230512225333050001100213300333004002305700130580012253330530011337000049001080124000666460a2444a6660a000420822a6660a0002208226609a6600860ae00460ae0026600660b000460b000244666e54004dd999ba548000cc12d4ccc14ccdc3801a400026ea12000153330533370e006900109ba84800854ccc14ccdc3801a400826ea12004153330533370e006900309ba84801854ccc14ccdc3801a401026ea1200813750900519825982a80c998259ba80083304b375000e660966e9c018cc12c014cc12cdd4802198259ba8337026eb4c154048dd6982a810198259ba8337026eb4c154044dd6982a80f998259ba8375a60aa60ac0180946eb80080200252f58a11c52e00882e57cefb05c3586f033d838717ba061a8a6db605f54979baf001bac304a304b02f48008dd698250051bad304a0093758609400603a6eb8c1280094ccc120cdc39bad304a0314800052000153330483370e6eb4c1280c5200214800854ccc120cdc39bad304a0314801052004153330483370e6eb4c1280c5200614801854ccc120cdc39bad304a031480205200814802854ccc120cdc3a99982419b87375a609406290000a40002a66609066e1cdd69825018a4004290010a99982419b87375a609406290020a40082a66609066e1cdd69825018a400c290030a99982419b87375a609406290040a401029005240002a6607066e1cdd6982500b9bad304a00915330383375e6e9cdd618250089ba7375860940062a6607066e3cdd718250081bae304a00215330383303701e01d153303832323253303b53303b3370e6607a050609a0386607a044609a0382a6607666e1ccdc09981e811182680f1981e814182680f19b81375a609a0146eb4c1340604cdc399b813303d022304d01d3303d028304d01d337026eb4c134024dd6982680b8a9981da9981d99b873303d001304d01e3038337026eb4c134028dd6982680c099b873303d001304d01d3038337026eb4c134024dd6982680b8a9981d99b8f375c609a0266eb8c13401454cc0eccdc39981e814001a40042a6607666e252000375a609a014266e252000375a609a0126eacc130c134c138004cc8c124894ccc11c004584c8c8c8c94ccc140cdc3a40000042600c60a600a2a6660a066e3cdd7182900080389829002898031829802982a00118278009baa304f305100132304f3051001304e001375c6096022052609405e266e1cdd6982500c1bad304a00a153330483370ea66609066e1cdd69825018a4000290000a99982419b87375a609406290010a40042a66609066e1cdd69825018a4008290020a99982419b87375a609406290030a400c2a66609066e1cdd69825018a4010290040a401490010991929981d19b87375a60980326eb4c13002c54cc0e94cc0e8cdd7982600b9826004899baf304c016304c008153303a3375e6e9cdd618260099ba73758609800a2a6607466e3cdd718260091bae304c004153303a33038027021153303a323232533304d3370e90010010991919299982819b8748000008528099baf374e0086e9c004c150008c13c004dd50020991919299982819b8748008008528099baf374e0086e9c004c150008c13c004dd5002182880118260009baa00213370e6eb4c130068dd698260061825982680f1825182600f0a99982419b8753330483370e6eb4c1280c5200014800054ccc120cdc39bad304a0314800852002153330483370e6eb4c1280c5200414801054ccc120cdc39bad304a0314801852006153330483370e6eb4c1280c520081480205200a4801054cc0e14cc0e0cdd7982500a9825003899baf304a014304a00615330383375e6e9cdd618250089ba7375860940062a6607066e3cdd718250081bae304a00215330385330383303602501f13303701e01d15330383370e6eb4c128060dd698250050a9981c19b89375a609401290605c01099b8948008dd698250048a99982419b8753330483370e6eb4c1280c5200014800054ccc120cdc39bad304a0314800852002153330483370e6eb4c1280c5200414801054ccc120cdc39bad304a0314801852006153330483370e6eb4c1280c520081480205200a4801854cc0e0cdc39bad304a017375a60940122a66070a6607066ebcc128054c12801c4cdd7982500a18250030a9981c19baf374e6eb0c128044dd39bac304a00315330385330383303602501f13303701e01d13370e6eb4c128060dd698250050a99982419b8753330483370e6eb4c1280c5200014800054ccc120cdc39bad304a0314800852002153330483370e6eb4c1280c5200414801054ccc120cdc39bad304a0314801852006153330483370e6eb4c1280c520081480205200a4802054cc0e0cdc39bad304a017375a60940122a66070a6607066ebcc128054c12801c4cdd7982500a18250030a9981c19b8f375c60940206eb8c12800854cc0e14cc0e0cc0d809407c4cc0dc0780744cdc39bad304a018375a60940142a6607094454cc0e0cdc39bad304a017375a60940122a66070a6607066ebcc128054c12801c4cdd7982500a18250030a9981c19b8f375c60940206eb8c12800854cc0e0cdd79ba7375860940226e9cdd618250018a9981c29981c1981b01280f89981b80f00e8a9981c19b89375a6094014905f4d06099b8948008dd6982500518250009824800982400098238009823000982280098220009821800982100098208009820000981f800981f000981f009181e000981d800981d000981c800981c000981b800981b000981a800981a00098198009819000981880098180009818004181698178021918169817800981618168041bab302b302c302d00232323232533302c3370e900200109919191919002a99981819b87480000084c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c94ccc188cdc39b8d001480e04c8c8c8c8c8c8c8c9265333064001149858c1ac00cdd6800983400098340019bae0013065001306500316375c00260c400260c4006660a846464646400aa6660c466e1d20000021323232323232324994ccc18c00452616306a00653330663370e900000109919299983419b87371a002901c0991924ca6660c80022930b18358018b1bae0013068001153330663370e900100109919299983419b87371a002901c0991924ca6660c80022930b18358018b1bae001306800116306a0023065001375400260c80022a6660c466e1d20020021323232323232323232324994ccc19800452616306d003375a00260d400260d40066eb4004c19c004c19c00cdd680098320008b183300118308009baa001375800260be00260be0066eb4004c170004c17000cdd6800982c800982c8019bad00130560013056003375a00260a600260a60066eb4004c140004c14000cdd6800982680098268019bad001304a001304a00653330463370e900000109919192999824a9982119b87001480004cdc3800a40702646464a66609866e24dc6800a4080264649329998240008a4c2c609e0062c6eb8004c130004c13001058dc68009bae001304800116304a002304500137540026088002608800ca66608066e1d20000021323232533304353303c3370e0029000099b87001480e04c8c8c94ccc118cdc49b8d001481004c8c9265333042001149858c12400c58dd7000982300098230020b1b8d001375c00260840022c6088004607e0026ea8004c0f8004c0f80194ccc0e8cdc3a40000042646464a66607aa6606c66e1c005200013370e002901c0991919299982019b89371a00290200991924ca6660780022930b18218018b1bae0013040001304000416371a0026eb8004c0f000458c0f8008c0e4004dd5000981c000981c003299981a19b87480000084c8c8c94ccc0dd4cc0c0cdc3800a4000266e1c00520381323232533303a337126e340052040132324994ccc0d800452616303d00316375c002607400260740082c6e34004dd7000981b0008b181c00118198009baa0013032001163034002302f0013754002605c0022c606000460560026ea8004c0a8c090c0b0004cc8c09c894ccc0940045854ccc0a8cdc39980e191bab302d302e302f001302c001003480084c0b00044c008c0b4004c0a403801cdd5991814981518158009814181498150009981119b8148008dd698138068031919191919299981499b87480100084c8c8c8c8c80154ccc0b4cdc3a400000426464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464a6660be66e1cdc6800a4070264646464646464649329998308008a4c2c60d00066eb4004c194004c19400cdd7000983100098310018b1bae001305f001305f00333051232323232005533305f3370e900000109919191919191924ca6660c00022930b1833803299983199b87480000084c8c94ccc194cdc39b8d001480e04c8c9265333061001149858c1a000c58dd700098328008a99983199b87480080084c8c94ccc194cdc39b8d001480e04c8c9265333061001149858c1a000c58dd700098328008b183380118310009baa00130610011533305f3370e900100109919191919191919191924ca6660c60022930b18350019bad00130670013067003375a00260c800260c80066eb4004c18400458c18c008c178004dd50009bac001305c001305c003375a00260b200260b20066eb4004c158004c15800cdd6800982980098298019bad00130500013050003375a002609a002609a0066eb4004c128004c12800cdd680098238009823803299982199b87480000084c8c8c94ccc1194cc0fccdc3800a4000266e1c005203813232325333049337126e340052040132324994ccc11400452616304c00316375c002609200260920082c6e34004dd700098228008b182380118210009baa00130410013041006533303d3370e90000010991919299982029981c99b87001480004cdc3800a40702646464a66608666e24dc6800a40802646493299981f8008a4c2c608c0062c6eb8004c10c004c10c01058dc68009bae001303f001163041002303c00137540026076002607600ca66606e66e1d20000021323232533303a5330333370e0029000099b87001480e04c8c8c94ccc0f4cdc49b8d001481004c8c9265333039001149858c10000c58dd7000981e800981e8020b1b8d001375c00260720022c6076004606c0026ea8004c0d4004c0d40194ccc0c4cdc3a40000042646464a666068a6605a66e1c005200013370e002901c0991919299981b99b89371a00290200991924ca6660660022930b181d0018b1bae0013037001303700416371a0026eb8004c0cc00458c0d4008c0c0004dd500098178008b181880118160009baa001302b00116302d00230280013754002604e60426052002604c604e0046eacc8c098c09cc0a0004c094c098004c098004cc078dd698118048011bac30223023302300237586042002604460400082c6044004603a0026ea8c074c078004c078010c070004c06c004c06c008526164bd68119b814800000488cdd79ba6002374c0024466ebcdd3980b8011ba730170012233301200200100314a044646660080066eb8c04c004dd71809980a000980a0009111999802001240004666600a00490003ad3756002006460046ea40048888cc038894ccc030004401454ccc044cdd79809180980080309802180b180980089801180a0008009180511299980400088020998029801980780098011808000a5eb815d01198040008010a515573e46010601000244a66600a66e20009200016133003002001300122253330053370e00490000980380089980199b8100248008c0200048c8c0088cc0080080048c0088cc0080080055cd2ab9d5742ae888c008dd5000aab9e1",
    };
  },
  {
    conf: {
      "title": "Config",
      "anyOf": [{
        "title": "Config",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          { "dataType": "integer", "title": "daoaction" },
          {
            "title": "poolnft",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "integer", "title": "poolfee" },
          { "dataType": "integer", "title": "treasuryfee" },
          {
            "dataType": "list",
            "items": {
              "title": "Referenced",
              "description":
                "Represent a type of object that can be represented either inline (by hash)\n or via a reference (i.e. a pointer to an on-chain location).\n\n This is mainly use for capturing pointers to a stake credential\n registration certificate in the case of so-called pointer addresses.",
              "anyOf": [{
                "title": "Inline",
                "dataType": "constructor",
                "index": 0,
                "fields": [{
                  "description":
                    "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                  "anyOf": [{
                    "title": "VerificationKeyCredential",
                    "dataType": "constructor",
                    "index": 0,
                    "fields": [{ "dataType": "bytes" }],
                  }, {
                    "title": "ScriptCredential",
                    "dataType": "constructor",
                    "index": 1,
                    "fields": [{ "dataType": "bytes" }],
                  }],
                }],
              }, {
                "title": "Pointer",
                "dataType": "constructor",
                "index": 1,
                "fields": [{ "dataType": "integer", "title": "slotNumber" }, {
                  "dataType": "integer",
                  "title": "transactionIndex",
                }, { "dataType": "integer", "title": "certificateIndex" }],
              }],
            },
            "title": "adminaddress",
          },
          {
            "title": "pooladdress",
            "description":
              "A Cardano `Address` typically holding one or two credential references.\n\n Note that legacy bootstrap addresses (a.k.a. 'Byron addresses') are\n completely excluded from Plutus contexts. Thus, from an on-chain\n perspective only exists addresses of type 00, 01, ..., 07 as detailed\n in [CIP-0019 :: Shelley Addresses](https://github.com/cardano-foundation/CIPs/tree/master/CIP-0019/#shelley-addresses).",
            "anyOf": [{
              "title": "Address",
              "dataType": "constructor",
              "index": 0,
              "fields": [{
                "title": "paymentCredential",
                "description":
                  "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                "anyOf": [{
                  "title": "VerificationKeyCredential",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{ "dataType": "bytes" }],
                }, {
                  "title": "ScriptCredential",
                  "dataType": "constructor",
                  "index": 1,
                  "fields": [{ "dataType": "bytes" }],
                }],
              }, {
                "title": "stakeCredential",
                "anyOf": [{
                  "title": "Some",
                  "description": "An optional value.",
                  "dataType": "constructor",
                  "index": 0,
                  "fields": [{
                    "description":
                      "Represent a type of object that can be represented either inline (by hash)\n or via a reference (i.e. a pointer to an on-chain location).\n\n This is mainly use for capturing pointers to a stake credential\n registration certificate in the case of so-called pointer addresses.",
                    "anyOf": [{
                      "title": "Inline",
                      "dataType": "constructor",
                      "index": 0,
                      "fields": [{
                        "description":
                          "A general structure for representing an on-chain `Credential`.\n\n Credentials are always one of two kinds: a direct public/private key\n pair, or a script (native or Plutus).",
                        "anyOf": [{
                          "title": "VerificationKeyCredential",
                          "dataType": "constructor",
                          "index": 0,
                          "fields": [{ "dataType": "bytes" }],
                        }, {
                          "title": "ScriptCredential",
                          "dataType": "constructor",
                          "index": 1,
                          "fields": [{ "dataType": "bytes" }],
                        }],
                      }],
                    }, {
                      "title": "Pointer",
                      "dataType": "constructor",
                      "index": 1,
                      "fields": [
                        { "dataType": "integer", "title": "slotNumber" },
                        { "dataType": "integer", "title": "transactionIndex" },
                        { "dataType": "integer", "title": "certificateIndex" },
                      ],
                    }],
                  }],
                }, {
                  "title": "None",
                  "description": "Nothing.",
                  "dataType": "constructor",
                  "index": 1,
                  "fields": [],
                }],
              }],
            }],
          },
          { "dataType": "bytes", "title": "treasuryaddress" },
          { "dataType": "integer", "title": "treasuryxwithdraw" },
          { "dataType": "integer", "title": "treasuryywithdraw" },
          { "dataType": "integer", "title": "nonce" },
        ],
      }],
    },
  },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolDaoV1DummyValidate;

export interface IRoyaltyPoolRoyaltyWithdrawPoolValidate {
  new (): Validator;
  conf: Data;
  action: Data;
}

export const RoyaltyPoolRoyaltyWithdrawPoolValidate = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "590d44590d410100003232323232323232323232323232323232323232323232225333014323232323253330193370e900200109919191919191919191919191919191919191919191919191919191919191919191919191919192998178100a9981799baf30420163042004153302f3370e646608044a66607e00229000099b80303e37566090608a0026004608c0020026eacc108054c8cc100894ccc0fc00452000133700607c6eacc120c114004c008c118004004dd598210018a9981799b873303237566084006608402690010a99817a9981799b89375a60840386eb4c10802854cc0bccdc49bad304201b375a60840122a6605e66e1cdd6982100099b81375a60840146eb4c10807054cc0bccdc39bad30423043001337026eb4c108024dd6982100d8a9981799b8733702660646eacc108054c108048dd6982100e198191bab30420033042012153302f3370e66e04cc0c8dd5982100a98210089bad304201b33032375660840066084022266e1ccc0c8dd5982100a9821008198191bab30420033042010153302f33372a6eb8c108064dd999ba548000cc0dcc108078cc0dcdd41bad3042304300603a375c6084608603c2a6605e66e1cc0ec0992004153302f33223375e6e9cc118008dd3982300099ba548000cc0dcc10804ccc0dcc108048cc0dcc108044cc0dcc108040cc0dcdd41bad304200f3303737506eb4c108038cc0dcdd41bad304200d3303737506eb4c108030cc0dcdd41bad304200b3303737506eb4c108004cc0dcdd41bad3042304300133037374e6eb0c108020cc0dcdd49bae30420073303737526eb8c108018cc0dcdd419b80375a6084608600c900101d0010a9981799b8f37286eb8c108064dd718210030991929981899b8f375c6088608a0046eb8c110c1140044cdc79bae3044002375c6088002608a60860286088608403a606064608660866086608660866062002608600264646464a66608466e1d20040021323232323200553330463370e900000109919191919191919191919191919191919191919191919191919191919191919191919191919191919191919191919191919299983c19b87371a002901c0991919191919191924ca6660f60022930b1840808019bad001307e001307e003375c00260f600260f60062c6eb8004c1e0004c1e000ccc1a88c8c8c8c80154ccc1e0cdc3a400000426464646464646493299983d0008a4c2c61000200ca6660f866e1d200000213232533307e3370e6e340052038132324994ccc1ec0045261630810100316375c00260fc0022a6660f866e1d200200213232533307e3370e6e340052038132324994ccc1ec0045261630810100316375c00260fc0022c61000200460f60026ea8004c1e800454ccc1e0cdc3a400400426464646464646464646493299983e8008a4c2c6106020066eb4004c20004004c2000400cdd6800983e800983e8019bad001307a00116307c002307700137540026eb0004c1d4004c1d400cdd6800983900098390019bad001306f001306f003375a00260d800260d80066eb4004c1a4004c1a400cdd6800983300098330019bad00130630013063003375a00260c000260c000ca6660b866e1d20000021323232533305f5330573370e0029000099b87001480e04c8c8c94ccc188cdc49b8d001481004c8c926533305f001149858c19400c58dd7000983100098310020b1b8d001375c00260bc0022c60c000460b60026ea8004c168004c1680194ccc158cdc3a40000042646464a6660b2a660a266e1c005200013370e002901c0991919299982e19b89371a00290200991924ca6660b20022930b182f8018b1bae001305c001305c00416371a0026eb8004c16000458c168008c154004dd5000982a000982a003299982819b87480000084c8c8c94ccc14d4cc12ccdc3800a4000266e1c005203813232325333056337126e340052040132324994ccc14c00452616305900316375c00260ac00260ac0082c6e34004dd700098290008b182a00118278009baa001304e001304e006533304a3370e900000109919192999826a9982299b87001480004cdc3800a40702646464a6660a066e24dc6800a4080264649329998268008a4c2c60a60062c6eb8004c140004c14001058dc68009bae001304c00116304e0023049001375400260900022c6094004608a0026ea8004c11000458c118008c104004dd50009820182080098200009820000999181d91299981d0008b0a99981f19b87330303237566082608460860026080002006900109820000898011820800981e80c010181e800981e000981d800981d000981c800981c000981b800981b000981a800981a000981980098190009818800981880099191919299981819b87480100084c8c8c8c8c80154ccc0d0cdc3a400000426464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464a6660cc66e1cdc6800a4070264646464646464649329998348008a4c2c60de0066eb4004c1b0004c1b000cdd7000983480098348018b1bae001306600130660033305823232323200553330663370e900000109919191919191924ca6660d00022930b1837003299983519b87480000084c8c94ccc1b0cdc39b8d001480e04c8c9265333069001149858c1bc00c58dd700098360008a99983519b87480080084c8c94ccc1b0cdc39b8d001480e04c8c9265333069001149858c1bc00c58dd700098360008b183700118348009baa0013068001153330663370e900100109919191919191919191924ca6660d60022930b18388019bad001306e001306e003375a00260d600260d60066eb4004c1a000458c1a8008c194004dd50009bac00130630013063003375a00260c000260c00066eb4004c174004c17400cdd6800982d000982d0019bad00130570013057003375a00260a800260a80066eb4004c144004c14400cdd680098270009827003299982519b87480000084c8c8c94ccc1354cc114cdc3800a4000266e1c005203813232325333050337126e340052040132324994ccc13400452616305300316375c00260a000260a00082c6e34004dd700098260008b182700118248009baa0013048001304800653330443370e900000109919192999823a9981f99b87001480004cdc3800a40702646464a66609466e24dc6800a4080264649329998238008a4c2c609a0062c6eb8004c128004c12801058dc68009bae0013046001163048002304300137540026084002608400ca66607c66e1d2000002132323253330415330393370e0029000099b87001480e04c8c8c94ccc110cdc49b8d001481004c8c9265333041001149858c11c00c58dd7000982200098220020b1b8d001375c00260800022c6084004607a0026ea8004c0f0004c0f00194ccc0e0cdc3a40000042646464a666076a6606666e1c005200013370e002901c0991919299981f19b89371a00290200991924ca6660760022930b18208018b1bae001303e001303e00416371a0026eb8004c0e800458c0f0008c0dc004dd5000981b0008b181c00118198009baa0013032001163034002302f0013754002605c605e002605c002605c6058605a002605a0026604a6eb4c0a804c038c0a8004c0a4004c0a0004c09c004c09cc094004c098004c8c8c8c94ccc094cdc3a40080042646464646400aa66605266e1d20000021323232323232323232324994ccc0b8004526163034003375c0026062002606200ca66605a66e1d200000213232323232323232323232323232533303b3370e6e340052038132323232323232324994ccc0f8004526163044003375a002608200260820066eb8004c0f8004c0f800c58dd7000981d800981d8019bad00130380013038003375a002606a002606a00ca66606266e1d20000021323232533303453302c3370e0029000099b87001480e04c8c8c94ccc0dccdc49b8d001481004c8c9265333034001149858c0e800c58dd7000981b800981b8020b1b8d001375c00260660022c606a00460600026ea8004c0bc00458c0c4008c0b0004dd500098158008b181680118140009baa0013027001163029002302400137540026046604860480066466e1cc088dd5000a4004646464a66604666e1d2002002153330233371e6eb8c0940052211c40a533ea4e3023c62912f029c7ad388bf3c2254e9c7fb3450024bc6e001323374a66604800290012400003c66e1cc07802520041616302700230220013754002646044604800260420026044604060420026042002660326eb4c078c07c01c008dd6180e980f180f0011bac301c001301d301b00516301d0023018001375460306032004603200660300022930b111998098010008018a502301330130012232333004003375c60260026eb8c04cc050004c050004888cccc01000920002333300500248001d69bab00100323002375200244446601c44a66601a002200a2a66602266ebcc048c04c0040184c010c058c04c0044c008c0500040048c028894ccc024004401c4cc010c00cc03c004c008c0400055d01198048008010a514bd70198021112999802000880109980199b8000248008c02c00520005573e44a66600a66e20009200016133003002001300122253330053370e00490000980380089980199b8100248008c0200048c8c0088cc0080080048c0088cc0080080055cd2ab9d5742ae888c008dd5000aab9e1",
    };
  },
  { conf: { "title": "Data", "description": "Any Plutus data." } },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolRoyaltyWithdrawPoolValidate;

export interface IRoyaltyPoolDepositValidate {
  new (): Validator;
  conf: {
    poolnft: { policy: string; name: string };
    x: { policy: string; name: string };
    y: { policy: string; name: string };
    lq: { policy: string; name: string };
    exFee: bigint;
    rewardPkh: string;
    stakePkh: string | null;
    collateralAda: bigint;
  };
  action: Data;
}

export const RoyaltyPoolDepositValidate = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "59085d59085a0100003232323232323232323232323232323232323232323232323232323222253330173232323232323232323232323253330243370ea66604866e1cdd698149815001a4000290000a4004900009919191919191919191919191919191919299981a99b87480080084c8c8c8c8c94cc0a4cdc399816808181f811240042a6605266ebcc0fc018c0fc02854cc0a4cdc399981c111299981e000880109980199b8000248008c10c00520000164801054cc0a54ccc0e8cdc38010008a511533303a33710004002266666604e026607e04000200400600a266666604e026607e04200400200800a266e254ccc0e8cdc48010008801080099816809981f80f999999813804181f00f8010021bad303e01d375a607c607e03666666604c00e607a03e0040066eb4c0f4070dd6981e981f00d19b81337026605401a607803a6eb4c0f0028dd6981e181e80499b813370266052018607603a6eb4c0ec028dd6981d80419b81483fbfffffffffffffffc04cc0a002cc0e806858c0f0008c0b8004dd5181b981c00d9bab323037303830390013036303700130370013302b375a606801e016606800260660026460666066605c00260660026464646464a66606066e1d20040021323232323200553330343370e900000109919191919191919191919191919191919191919191919191919191919191919191919191919191919191919191919191919299983319b87371a002901c0991919191919191924ca6660d80022930b18390019bad001306f001306f003375c00260d800260d80062c6eb8004c1a4004c1a400ccc8c184894ccc190004417c4cc170c00cc1a8004c008c1ac0048c8c8c8c80154ccc198cdc3a40000042646464646464649329998358008a4c2c60e200ca6660d466e1d200000213232533306c3370e6e340052038132324994ccc1b000452616307200316375c00260de0022a6660d466e1d200200213232533306c3370e6e340052038132324994ccc1b000452616307200316375c00260de0022c60e200460c60026ea8004c1ac00454ccc198cdc3a40040042646464646464646464649329998370008a4c2c60e80066eb4004c1c4004c1c400cdd6800983700098370019bad001306b00116306d002305f00137540026eb0004c198004c19800cdd6800983180098318019bad00130600013060003375a00260ba00260ba0066eb4004c168004c16800cdd6800982b800982b8019bad00130540013054003375a00260a200260a200ca66609466e1d20000021323232533304d5330453370e0029000099b87001480e04c8c8c94ccc140cdc49b8d001481004c8c9265333050001149858c15800c58dd7000982980098298020b1b8d001375c002609e0022c60a200460860026ea8004c12c004c12c0194ccc110cdc3a40000042646464a66608ea6607e66e1c005200013370e002901c0991919299982519b89371a00290200991924ca6660940022930b18280018b1bae001304d001304d00416371a0026eb8004c12400458c12c008c0f4004dd500098228009822803299981f19b87480000084c8c8c94ccc1054cc0e4cdc3800a4000266e1c005203813232325333044337126e340052040132324994ccc11000452616304a00316375c002608e002608e0082c6e34004dd700098218008b1822801181b8009baa001303f001303f00653330383370e90000010991919299981da9981999b87001480004cdc3800a40702646464a66607c66e24dc6800a40802646493299981f0008a4c2c60880062c6eb8004c104004c10401058dc68009bae001303d00116303f0023031001375400260720022c6076004605a0026ea8004c0d400458c0dc008c0a4004dd5000981898181819800981818188011bab32303030313032001302f3030001303000133024375a605a0120086644646464a666058646464a66605e66e1d2002002132323253330323370e90000010a5013375e6e9c010dd3800981c80118158009baa007132323253330323370e90010010a5013375e6e9c010dd3800981c80118158009baa00730360023028001375400220042c6464646464a66606066e1d200000213232323253330343370e9000001099191919299981c19b8748008008584cdd2a4000002607e00460620026ea8004c0e400458c0ec008c0b4004dd5000981a800899ba5480080a8c0dc008c0a4004dd5000981898191819800981818190029bab323232533302d3370e90010010b0a99981699b8f375c606400200c260646066606800e2c6068004604c0026ea8004c8c0bcc0c4004c0b8c0c000cdd718160051816004998111bad302b005001375860540046eb0c0a40084cc88cc8c094894ccc0a00045280a99981499baf302e00100314a226004605e0026ea4004008dd61814981298150009bae3029007302930290013029302700b30270013026001302600a30240013023001302200130210013020001301f001301f004301e001149858888888cdc499b833370466e0401000c008004cc030018014888888c8cdc199b825333019323253300a3371e6eb8c080c084008dd718101810800899b8f375c60400046eb8c080004c08402cc0800184cdc099b8100100300210010040053300b0060052233301300200100314a066e9520003300637520026600c6ea40040252201002232333004003375c602a0026eb8c054c058004c058004888cccc01000920002333300500248001d69bab00100323002375200244446601844a66601e002200a2a66602066ebcc02cc0540040184c010c060c0540044c008c0580040055d01198048008010a514bd702ab9d2253330063371000490000b0998018010009800911299980319b87002480004c02c0044cc00ccdc0801240046018002464600446600400400246004466004004002aae7d5cd118031801000918029801000918021801000918019801800aba15744460046ea800555cf01",
    };
  },
  {
    conf: {
      "title": "DepositConfig",
      "anyOf": [{
        "title": "Config",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          {
            "title": "poolnft",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "x",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "y",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "lq",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "integer", "title": "exFee" },
          { "dataType": "bytes", "title": "rewardPkh" },
          {
            "title": "stakePkh",
            "anyOf": [{
              "title": "Some",
              "description": "An optional value.",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes" }],
            }, {
              "title": "None",
              "description": "Nothing.",
              "dataType": "constructor",
              "index": 1,
              "fields": [],
            }],
          },
          { "dataType": "integer", "title": "collateralAda" },
        ],
      }],
    },
  },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolDepositValidate;

export interface IRoyaltyPoolRedeemValidate {
  new (): Validator;
  conf: {
    poolnft: { policy: string; name: string };
    x: { policy: string; name: string };
    y: { policy: string; name: string };
    lq: { policy: string; name: string };
    exFee: bigint;
    rewardPkh: string;
    stakePkh: string | null;
  };
  action: Data;
}

export const RoyaltyPoolRedeemValidate = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "5908fa5908f70100003232323232323232323232323232323232323232323232323232323232222533301c3232323232323232323232323253330293370e00290000991919191919191919191919191919299981c19b87480080084c8c8c8c94cc094cdc399814991bab303e303f3040001303d303e00e303d01f4800854cc094cc88cdd79ba73041002374e6082002607a00a607a0102a6604a66e1cccc0d88894ccc0e800440084cc00ccdc00012400460820029000009a40082a6604aa6604a66e24ccccc09000c004c8dd5981f181f9820000981e981f007181e80f19b80375a607a0186eb4c0f4028dd6981e981f801099b8933333024003001323756607c607e6080002607a607c01c607a03a66e00dd6981e8059bad303d303e00a375a607a607c607e004266e24cdc01bad303d303b303f002004302d01033028323756607a607c607e0026078607a00e607803666644464646464a6660826464a6605866e3cdd7182218228011bae3044304500113371e6eb8c110008dd718220009822816982200389998169ba800237500066ea000854ccc104c8c94cc0b0cdc79bae30443045002375c6088608a002266e3cdd718220011bae3044001304502d3044006133302d37500086ea0004dd400089998169ba800437500066ea120003370200400866e0400800ccc0ac04800ccc0a804400cc0ec070c0ec06c008cdc0a41fdfffffffffffffffe026604c646eacc0ecc0f0c0f4004c0e8c0ec02cc0e8064cdc09814991bab303a303b303c0013039303a004375a607202e2c6076004605a0026ea8c0d8c0dc064c0dc004cc0acdd6981a007805181a00098198009918199819981680098198009919191919299981a19b87480100084c8c8c8c8c80154ccc0e0cdc3a400000426464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464a6660d466e1cdc6800a4070264646464646464649329998360008a4c2c60e40066eb4004c1bc004c1bc00cdd7000983600098360018b1bae001306900130690033323061225333064001105f13305c3003306a0013002306b001232323232005533306a3370e900000109919191919191924ca6660d60022930b1838803299983719b87480000084c8c94ccc1c0cdc39b8d001480e04c8c926533306c001149858c1c800c58dd700098378008a99983719b87480080084c8c94ccc1c0cdc39b8d001480e04c8c926533306c001149858c1c800c58dd700098378008b183880118318009baa001306b0011533306a3370e900100109919191919191919191924ca6660dc0022930b183a0019bad00130710013071003375a00260dc00260dc0066eb4004c1ac00458c1b4008c17c004dd50009bac00130660013066003375a00260c600260c60066eb4004c180004c18000cdd6800982e800982e8019bad001305a001305a003375a00260ae00260ae0066eb4004c150004c15000cdd680098288009828803299982719b87480000084c8c8c94ccc1454cc114cdc3800a4000266e1c005203813232325333054337126e340052040132324994ccc14000452616305600316375c00260a600260a60082c6e34004dd700098278008b182880118218009baa001304b001304b00653330483370e900000109919192999825a9981f99b87001480004cdc3800a40702646464a66609c66e24dc6800a4080264649329998250008a4c2c60a00062c6eb8004c134004c13401058dc68009bae001304900116304b002303d0013754002608a002608a00ca66608466e1d2000002132323253330455330393370e0029000099b87001480e04c8c8c94ccc120cdc49b8d001481004c8c9265333044001149858c12800c58dd7000982380098238020b1b8d001375c00260860022c608a004606e0026ea8004c0fc004c0fc0194ccc0f0cdc3a40000042646464a66607ea6606666e1c005200013370e002901c0991919299982119b89371a00290200991924ca66607c0022930b18220018b1bae0013041001304100416371a0026eb8004c0f400458c0fc008c0c4004dd5000981c8008b181d80118168009baa0013035001163037002302900137540026062605e60660026060606200260620026604a6eb4c0b8028010cc88c8c8c94ccc0c4c8c8c94ccc0d0cdc3a40040042646464a66606e66e1d200000214a0266ebcdd38021ba7001303a002302c001375400e2646464a66606e66e1d200200214a0266ebcdd38021ba7001303a002302c001375400e606e00460520026ea8004400858c8c8c8c8c94ccc0d4cdc3a4000004264646464a66607266e1d2000002132323232533303d3370e90010010b099ba548000004c100008c0c8004dd5000981d0008b181e00118170009baa001303600113374a9001015981c00118150009baa001303230333034001303130330053756646464a66606466e1d200200216153330323371e6eb8c0cc0040184c0ccc0d0c0d401c58c0d4008c09c004dd50009918181819000981798188019bae302d00a302d302e00a33023375a605800c0026eb0c0ac00cdd618150018999119918131129998148008a501533302e3375e605e00200629444c008c0c0004dd48008011bac302a3025302b002375c605400ea66605066e1cdd698149815001a4000290000a4004605260520026052604e014604e002604c002604c012604800260460026044002604200260400026040008603e0022930b1111119b833370400866e04cc02800c00800401488ccc06400800400c52819ba548000cc020dd4803998041ba900700b2223374a900019805001998050011980500080691191998020019bae3017001375c602e60300026030002444666600800490001199980280124000eb4dd5800801918011ba900122223300e2253330110011005153330163375e601a602e00200c260086034602e0022600460300020024a66601600229000099980819baf3007301100137520046eb4c050c044dd5980a1808800a4000910100574046601a00200429452f5c0aae74894ccc028cdc4001240002c2660060040026002444a66601466e1c00920001300b0011330033370200490011806000919180111980100100091801119801001000aab9f2300730020012300630020012300530020012300430040015734ae855d1118011baa0015573c1",
    };
  },
  {
    conf: {
      "title": "RedeemConfig",
      "anyOf": [{
        "title": "Config",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          {
            "title": "poolnft",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "x",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "y",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "lq",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "integer", "title": "exFee" },
          { "dataType": "bytes", "title": "rewardPkh" },
          {
            "title": "stakePkh",
            "anyOf": [{
              "title": "Some",
              "description": "An optional value.",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes" }],
            }, {
              "title": "None",
              "description": "Nothing.",
              "dataType": "constructor",
              "index": 1,
              "fields": [],
            }],
          },
        ],
      }],
    },
  },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolRedeemValidate;

export interface IRoyaltyPoolFeeSwitchValidate {
  new (): Validator;
  action: Data;
}

export const RoyaltyPoolFeeSwitchValidate = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "59112259111f010000323232323232323232323232323232323232323232323232322253330153232323253330193370e900200109919191919191919191919191919191919191919191919191919191919191919191919191919191929981919b87333040222533304000110021330033370000490011824000a400004c90020a99819299981e80f0a51133223330430020014a06ae8cc114078cdd79821982200f1ba948810015330325330323375e608803060880182a6606466ebcc11005cc11002c54cc0c8cdd7982200b18220050a9981919baf3044015304400915330323375e6088024608800c2a6606466ebcc110034c1100044cdd798221822806982218228008a9981919b8948028ccc8c1048894ccc10400440084cc00ccc010008c120004c124004894ccc10ccc8c10c894ccc1080045280a99982319baf304900100314a22600460940026ea40040984cdc0001240042004900025eb1411c3703634e3e34c0e7fd9a7348ad213f9272279535c73f4ec00efe00bd00811cf3a12554ca0ccd1a220b1de839a1940bc1006f08768b636fa98c66c900811c518a9c32deedc0b82604972692a2b7eb6c10b020d77c3c72e764b15600811c68aa59a87dbdbf8f78386dc6b83e63149d8c13939a1b0cda39707f9a00811c1826edf8011dc6a084214b87a4ff690665692f4243e5bb5ff5aad53600811cd350803d45e327f8808469d5dde9e0f6fdc6e6637d85ed44cc37a12c000a99982099b8753330413370e6eb4c110c1180b9200014800054ccc104cdc39bad3044304602e4800852002153330413370e6eb4c110c1180b9200414801054ccc104cdc39bad3044304602e4801852006153330413370e6eb4c110c1180b920081480205200a4800054cc0c8cdc39bad3044013375a608800e2a6606466ebcc11003cc11000c54cc0c8cdd7982200718220010a998191981880d00c8a9981919191929981aa9981a99b873303702430470183303701e304701815330353370e66e04cc0dc078c11c068cc0dc090c11c068cdc09bad3047008375a608e028266e1ccdc09981b80f182380c9981b812182380c99b81375a608e00e6eb4c11c04c54cc0d54cc0d4cdc39981b800982380d181919b81375a608e0106eb4c11c0504cdc39981b800982380c981919b81375a608e00e6eb4c11c04c54cc0d4cdc79bae3047011375c608e00a2a6606a66e1ccc0dc09000d200215330353371290001bad304700813371290001bad30470073756608c608e6090002664608444a6660820022c264646464a66609266e1d200000213006304d005153330493371e6eb8c13000401c4c1300144c018c134014c138008c124004dd518249825800991824982580098240009bae304500f02630443042304602e13370e6eb4c110050dd698220040a99982099b8753330413370e6eb4c110c1180b9200014800054ccc104cdc39bad3044304602e4800852002153330413370e6eb4c110c1180b9200414801054ccc104cdc39bad3044304602e4801852006153330413370e6eb4c110c1180b920081480205200a480084c8c94cc0d0cdc39bad3046015375a608c0122a66068a6606866ebcc11804cc11801c4cdd7982300918230030a9981a19baf3046011304600515330343375e608c020608c0082a660686606404603a2a66068646464a66608c66e1d2002002132323253330493370e90000010a5013375e6e9c010dd3800982700118248009baa004132323253330493370e90010010a5013375e6e9c010dd3800982700118248009baa004304b00230460013754004266e1cdd6982300b1bad304600a3045304701a3044304601a153330413370ea66608266e1cdd69822182301724000290000a99982099b87375a6088608c05c90010a40042a66608266e1cdd69822182301724008290020a99982099b87375a6088608c05c90030a400c2a66608266e1cdd69822182301724010290040a401490020a9981929981919baf3044011304400513375e608802060880082a6606466ebcc11003cc11000c54cc0c8cdd7982200718220010a998192998191981801080d89981880d00c8a9981919b87375a60880286eb4c11002054cc0c8cdc49bad304400748302e0084cdc4a40046eb4c11001c54ccc104cdc3a99982099b87375a6088608c05c90000a40002a66608266e1cdd69822182301724004290010a99982099b87375a6088608c05c90020a40082a66608266e1cdd6982218230172400c290030a99982099b87375a6088608c05c90040a4010290052400c2a6606466e1cdd698220099bad304400715330325330323375e6088022608800a266ebcc110040c11001054cc0c8cdd7982200798220018a998192998191981801080d89981880d00c899b87375a60880286eb4c11002054ccc104cdc3a99982099b87375a6088608c05c90000a40002a66608266e1cdd69822182301724004290010a99982099b87375a6088608c05c90020a40082a66608266e1cdd6982218230172400c290030a99982099b87375a6088608c05c90040a401029005240102a6606466e1cdd698220099bad304400715330325330323375e6088022608800a266ebcc110040c11001054cc0c8cdd7982200718220010a998192998191981801080d89981880d00c899b87375a60880286eb4c11002054cc0c9288a9981919b87375a60880266eb4c11001c54cc0c94cc0c8cdd798220089822002899baf3044010304400415330323375e608801c60880042a6606466ebcc11003cc11000c54cc0c94cc0c8cc0c008406c4cc0c406806454cc0c8cdc49bad3044008482fa68304cdc4a40046eb4c110020c110004c10c004c0fcc108004c104004c100004c0fc004c0f8004c0f4004c0f0004c0ec004c0e8004c0e8040c0e0004c0dc004c0ccc0d8004c0d4004c0d0004c0cc004c0c8004c0c4004c0c0004c0bc004c0b8004c0b8020c0acc0b4010c8c0acc0b4004c0a8c0ac020dd598149815181580119191919299981499b87480100084c8c8c8c8c80154ccc0b4cdc3a400000426464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464646464a6660be66e1cdc6800a4070264646464646464649329998310008a4c2c60d20066eb4004c198004c19800cdd7000983180098318018b1bae0013060001306000333052232323232005533305f3370e900000109919191919191924ca6660c20022930b1834003299983199b87480000084c8c94ccc194cdc39b8d001480e04c8c9265333062001149858c1a400c58dd700098330008a99983199b87480080084c8c94ccc194cdc39b8d001480e04c8c9265333062001149858c1a400c58dd700098330008b183400118318009baa00130620011533305f3370e900100109919191919191919191924ca6660c80022930b18358019bad00130680013068003375a00260ca00260ca0066eb4004c18800458c190008c17c004dd50009bac001305d001305d003375a00260b400260b40066eb4004c15c004c15c00cdd6800982a000982a0019bad00130510013051003375a002609c002609c0066eb4004c12c004c12c00cdd680098240009824003299982199b87480000084c8c8c94ccc1194cc100cdc3800a4000266e1c005203813232325333049337126e340052040132324994ccc11800452616304d00316375c002609400260940082c6e34004dd700098230008b182400118218009baa00130420013042006533303d3370e90000010991919299982029981d19b87001480004cdc3800a40702646464a66608666e24dc6800a4080264649329998200008a4c2c608e0062c6eb8004c110004c11001058dc68009bae0013040001163042002303d00137540026078002607800ca66606e66e1d20000021323232533303a5330343370e0029000099b87001480e04c8c8c94ccc0f4cdc49b8d001481004c8c926533303a001149858c10400c58dd7000981f000981f0020b1b8d001375c00260740022c6078004606e0026ea8004c0d8004c0d80194ccc0c4cdc3a40000042646464a666068a6605c66e1c005200013370e002901c0991919299981b99b89371a00290200991924ca6660680022930b181d8018b1bae0013038001303800416371a0026eb8004c0d000458c0d8008c0c4004dd500098180008b181900118168009baa001302c00116302e002302900137540026050604c6054002664604844a6660460022c2a66604e66e1ccc068c8dd59815981618168009815000801a400426054002260046056002604e604a60520220106eacc8c09cc0a0c0a4004c098c09cc0a0004cc07ccdc0a40046eb4c094c098c09c03c01cc8c8c8c8c94ccc098cdc3a40080042646464646400aa66605466e1d200000213232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232533305c3370e6e340052038132323232323232324994ccc17c004526163066003375a00260c600260c60066eb8004c180004c18000c58dd7000982e800982e80199827919191919002a99982e19b87480000084c8c8c8c8c8c8c926533305e001149858c1940194ccc180cdc3a400000426464a6660c466e1cdc6800a40702646493299982f8008a4c2c60cc0062c6eb8004c18c00454ccc180cdc3a400400426464a6660c466e1cdc6800a40702646493299982f8008a4c2c60cc0062c6eb8004c18c00458c194008c180004dd5000982f8008a99982e19b87480080084c8c8c8c8c8c8c8c8c8c9265333061001149858c1a000cdd6800983280098328019bad00130620013062003375a00260be0022c60c200460b80026ea8004dd6000982d000982d0019bad00130570013057003375a00260a800260a80066eb4004c144004c14400cdd6800982700098270019bad001304b001304b003375a002609000260900066eb4004c114004c1140194ccc100cdc3a40000042646464a666086a6607a66e1c005200013370e002901c0991919299982319b89371a00290200991924ca6660860022930b18250018b1bae0013047001304700416371a0026eb8004c10c00458c114008c100004dd5000981f800981f803299981d19b87480000084c8c8c94ccc0f54cc0dccdc3800a4000266e1c005203813232325333040337126e340052040132324994ccc0f400452616304400316375c002608200260820082c6e34004dd7000981e8008b181f801181d0009baa0013039001303900653330343370e90000010991919299981ba9981899b87001480004cdc3800a40702646464a66607466e24dc6800a40802646493299981b8008a4c2c607c0062c6eb8004c0ec004c0ec01058dc68009bae0013037001163039002303400137540026066002606600ca66605c66e1d20000021323232533303153302b3370e0029000099b87001480e04c8c8c94ccc0d0cdc49b8d001481004c8c9265333031001149858c0e000c58dd7000981a800981a8020b1b8d001375c00260620022c6066004605c0026ea8004c0b400458c0bc008c0a8004dd500098148008b181580118130009baa001302530233027001302430250023756646048604a604c002604660480026048002660366eb4c084c088c08c02c00cdd6181019181118111811180f80098108019bac301f0023758603c004603c603c002603c60380082c603c00460320026ea8c064c068004c0680045261623370290000009119baf374c0046e9800488cdd79ba73017002374e602e00244666022004002006294088c8ccc01000cdd718098009bae3013301400130140012223333004002480008cccc014009200075a6eac00400c8c008dd480091111980691299980600088028a99980819baf3012301300100613004301630130011300230140010012300922533300800110041330053003300f001300230100014bd702ba023300700100214a2aae7c894ccc014cdc4001240002c2660060040026002444a66600a66e1c0092000130080011330033370200490011804800919180111980100100091801119801001000ab9a2300430040015573aae855d1118011baa0015573d",
    };
  },
  { action: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IRoyaltyPoolFeeSwitchValidate;

export interface IFactoryValidateFactory {
  new (): Validator;
  _: Data;
}

export const FactoryValidateFactory = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "5908890100003232323232323232322253330053232323253330093370e90021804000899191919191919299980819b8748000c03c0044c8c8c8c8c8c8c94ccc05ccdc3a4008602c0022646464646464646464646464646464646464646464a66605866e1d2000302b0011323232323232323253330343370e90021819800899191919191919191919299981f19b8748000c0f40044c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c94ccc158cdc3a400060aa002264646464646464646464646464a6660c6a6660c6a6660c6a6660c6a6660c6a6660c6a6660c603c20202940401452808020a50100f14a020062940400852808008a503370e664600200244a6660ce00229000099b8048008cc008008c1a800415520043375e04066e952000330653374a9000198329ba90394bd7019832a6103d87a80004bd7019b8733230010012253330650011480004cdc0240046600400460d0002646600200203c44a6660ca002297ae01330663063306700133002002306800148008cdd7818260150d8799fd87a9f581ccb684a69e78907a9796b21fc150a758af5f2805e5ed5d5a8ce9f76f1ffd8799fd8799fd87a9f581cb2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b07ffffffff00533305e3370e00c002266e1c009200214a066601e05891011c6e917b8b965078a39804a6313e5be73535612421acd70aa83f0ec20000372400666601c05691011cd8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add06003724002660046eb8c108c164100cc008dd99ba803e3766980101010033001375c608260b007e660026eccdd401e9bb337500044466e28008004cdc0a41fdfffffffffffffffe020026eb4c170004c15000458c94ccc158cdc4000a4000298103d87a800015333056337120029001099ba548000cc168dd4000a5eb804cdd2a4000660b46ea0c8c8ccc00400400c0088894ccc168cdc48010008801099980180180099b833370000266e0c01400520043370666e000052002480112f5c066e0801000ccdc4815002a99982999b8700233702008904071516400899b8700100314a066600804200e00a66600604001401066600407800a00666600207601000c444646464a6660a866e1d20020011480004dd6982c9829001182900099299982999b8748008004530103d87a8000132323300100100222533305900114c103d87a8000132323232533305a3371e014004266e9520003305e375000297ae0133006006003375a60b60066eb8c164008c174008c16c004dd5982c18288011828800991980080080211299982b0008a6103d87a800013232323253330573371e010004266e9520003305b374c00297ae0133006006003375660b00066eb8c158008c168008c160004dd7182900098290011bae30500013048026375c609c002609c0046eb8c130004c110090cdd780080819ba548000cc120030cc120088cc120080cc120020cc1213001051a0001831c00330484c102183200330484c102183200330484c1010000330484c1010000330484c1010000330484c1010000330484c1289fd8799fd87a9f581c66e711a4bf9ddf46ff239143870b6893055a4fd4dea9f99fed6665cdffffff00330484c11e581c75c4570eb625ae881b32a34c52b159f6f3f3f2c7aaabf5bac468813300330484c12258202006b2ffddc15d9e6c659e570438fbeeebcf83f30118e64d9a55894e2172e34e00330484c10100004bd701bab304800130480023046001303e0013044001303c0011633323001001222533304300214c0103d87a80001323253330423370e0069000099ba548000cc1180092f5c0266600a00a00266e0400d20023047003304500203048008c104004c104004c100004c0fc008c0f4004c0d4008c94ccc0dccdc3a40000022646464646464646464646464646464646464646464646464646464646464a6660b060b60042646464646493191980080080611299982f0008a4c2646600600660c40046464a6660ba66e1d20000011323253330623065002132498c94ccc180cdc3a400000226464a6660ca60d00042930b1bae3066001305e002153330603370e900100089919299983298340010a4c2c6eb8c198004c17800858c17800458c18c004c16c00854ccc174cdc3a40040022646464646464a6660cc60d20042930b1bad30670013067002375a60ca00260ca0046eb4c18c004c16c00858c16c004c180004c0e8068c0e406cc0e0070c0dc07458dd6982c800982c8011bae30570013057002375c60aa00260aa0046eb0c14c004c14c008dd6982880098288011bad304f001304f002375a609a002609a0046eb4c12c004c12c008dd6982480098248011bad30470013047002375a608a002608a0046086002608600460820026082004607e002607e004607a002606a0042c606a002607400260640022c607000260700046eacc0d8004c0d8008c0d0004c0b0004c0c8004c0a800458c074078dd6981780098178011bae302d001302d001302c001302b0023029001302900230270013027001301e0083253330203370e90000008991919191919191919191919191919191919299981a981c001099191924c602c01e602a02060280222c6eb8c0d8004c0d8008dd7181a000981a0011bad30320013032002375c606000260600046eb4c0b8004c0b8008dd69816000981600118150009815001181400098140011813000980f0040b180f003919299981019b87480000044c8c8c8c94ccc09cc0a800852616375c605000260500046eb8c098004c07800858c078004dd6981100098110011810000980c1800980c0049180f800980e800980a8008b180d800980d8011bab3019001301900130103017301830100013016001300e0011630010052533301200114c0103d87a800013374a900019809980a000a5eb80dd61809000980900098088011bac300f001300700316300d001300d002300b001300300114984d9588c014dd5000918019baa0015734aae7555cf2ab9f5740ae855d11",
    };
  },
  { _: { "title": "Data", "description": "Any Plutus data." } },
) as unknown as IFactoryValidateFactory;

export interface IDepositDeposit {
  new (): Validator;
  datum: {
    poolNft: { policy: string; name: string };
    redeemer: string;
    minExpectedLpAmount: bigint;
  };
  action: {
    ApplyOrder: {
      redeemerInIx: bigint;
      redeemerOutIx: bigint;
      poolInIx: bigint;
    };
  } | "CancelOrder";
}

export const DepositDeposit = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "5904bc0100003232323232323223232322323225333009323232533300c300a300d375400e26464646464646464646464a66602e602a60306ea80044c8c8c8c8c8c8c94ccc078c070c07cdd5000899191919299981119b8748010c08cdd50008992999811981098121baa00113232323232323232323232323232323232323232323232323232533304030430021323232323232323232323253330483046304937540022646464646464a66609c609066600605c0660642a66609c0042002294052819192999827982698281baa00113371e0886eb8c150c144dd5000899baf300230513754606260a26ea80e0014c004c140dd5180098281baa00623053001337120666660026eacc144c14800c01c018888c94ccc13cc124c140dd50008a400026eb4c150c144dd5000992999827982498281baa00114c103d87a800013233001001375660aa60a46ea8008894ccc150004530103d87a8000132323253330543371e00e6eb8c15400c4c104cc160dd4000a5eb804cc014014008dd6982a801182c001182b00099198008008021129998298008a6103d87a8000132323253330533371e00e6eb8c15000c4c100cc15cdd3000a5eb804cc014014008dd5982a001182b801182a800982780098259baa001304d304a37540022c6606406a0726eb8c12cc130008dd7182500098231baa018330050092375a0026600801846eb8004c0f8054cc0080588dd68009980080b9181e8009119198008008019129998228008a4c2646600600660920046006608e00260740322c6eb4c104004c104008dd6181f800981f8011bae303d001303d0023758607600260760046eb4c0e4004c0e4008dd6981b800981b8011bad3035001303500232533303230310011533302f3029303000114a22a66605e605a606000229405858dd518198009819801181880098188011bac302f001302f0023758605a002605a0046eb4c0ac004c0ac008c0a4004c094dd50008b181398121baa00116302630270023756604a002604a60426ea8c004c084dd5181218109baa002230243025001163300800c00e375c604260440046eb8c080004c070dd5180f8011bad301e301f301f001301a375402e603860326ea800458cc004014dd6980d8051800800911299980d0010a60103d87a80001323253330193017003130063301d0024bd70099980280280099b8000348004c07800cc070008dd2a40006eb0c05cc060c060008dd6180b00098091baa007375a6028602a0046eb4c04c004c04c004c038dd5003899198008008019129998088008a50132533300f3371e6eb8c050008010528899801801800980a0009bae30103011300d37540146eb0c03cc040c040c040c040c040c040c040c040c030dd5000980718059baa00114984d958c94ccc020c0180044c8c8c8c8c8c94ccc044c05000852616375a602400260240046eb4c040004c040008dd6980700098051baa0031533300830020011533300b300a37540062930b0b18041baa002370e90012999802180118029baa0031323232323232533300d3010002132498c01c01458dd6980700098070011bae300c001300c002300a001300637540062c4a6660086004600a6ea80044c8c8c8c94ccc02cc03800852616375c601800260180046eb8c028004c018dd50008b1b87480015cd2ab9d5573caae7d5d02ba15745",
    };
  },
  {
    datum: {
      "title": "DepositData",
      "description": "AMM-orders data.",
      "anyOf": [{
        "title": "DepositData",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          {
            "title": "poolNft",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "bytes", "title": "redeemer" },
          { "dataType": "integer", "title": "minExpectedLpAmount" },
        ],
      }],
    },
  },
  {
    action: {
      "title": "OrderAction",
      "description": "Order action types.",
      "anyOf": [{
        "title": "ApplyOrder",
        "dataType": "constructor",
        "index": 0,
        "fields": [{ "dataType": "integer", "title": "redeemerInIx" }, {
          "dataType": "integer",
          "title": "redeemerOutIx",
        }, { "dataType": "integer", "title": "poolInIx" }],
      }, {
        "title": "CancelOrder",
        "dataType": "constructor",
        "index": 1,
        "fields": [],
      }],
    },
  },
) as unknown as IDepositDeposit;

export interface IPoolValidatePool {
  new (daoVotingWitness: string): Validator;
  inputDatum: {
    poolNft: { policy: string; name: string };
    n: bigint;
    tradableAssets: Array<{ policy: string; name: string }>;
    tradableTokensMultipliers: Array<bigint>;
    lpToken: { policy: string; name: string };
    lpFeeIsEditable: boolean;
    amplCoeff: bigint;
    lpFeeNum: bigint;
    protocolFeeNum: bigint;
    daoStabeProxyWitness: Array<string>;
    treasuryAddress: string;
    protocolFees: Array<bigint>;
    inv: bigint;
  };
  redeemer: {
    poolInIx: bigint;
    poolOutIx: bigint;
    action:
      | { AMMAction: { optionInt0: bigint; optionInt1: bigint } }
      | "PDAOAction";
  };
}

export const PoolValidatePool = Object.assign(
  function (daoVotingWitness: string) {
    return {
      type: "PlutusV2",
      script: applyParamsToScript(
        applyDoubleCborEncoding(
          "590e600100003232323232323223223232323232322322533300d3232323232325333013300c30143754008264646464646464646464a66603a6032603c6ea80044c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c94ccc0d0c0c0c0d4dd500089919191919299981c99b8748010c0e8dd5001099191919191919191919191919191919191919299982619baf03902e1533304c0071533304c0041533304c0031533304c002100114a029405280a5014a0a666096608e60986ea80d04c8ccc8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c888c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c94ccc234054ccc23404cdc38111bad3092010151533308d013370e0400262a66611a0266ebcdd380f1ba701113371e03801e29405280a50100114a0a66611802666118026110020129412889919191919191919299984a009999981d8048188058050098a99984a008038a99984a008018a99984a0080108008a5014a0294052819b8733702900000819b833370466e040c807ccdc0a41fdfffffffffffffffffffffffffffffffffe0e02203e666660726607464666600200200e0066607200a466e0ccdc119b82031029001005222253330990100314bd7009919299984d808020a5eb804c8c94ccc2740401452f5c026613c026ea0cdc099b81004002375a613e0200a666601001000600261400200a613e0200a6eb4c27404010c27404010dd6984d8080181581780480401899baf374e0126e9c004cc0d80088cdc199b823370405c04800200466e08cdc12401060ec05890604d0619981e0011981a004119b833370400205e03644a6661200266e200040084cdc0801000899b81001002333330343303500102602a00400302d330350070191323232533308f01308b01309001375400226464a66612202611a026124026ea80044c8c94ccc24c04c23c04c25004dd500089919299984a80984880984b009baa00113232533309701309301309801375400226464a66613202612a026134026ea80044c8c8c8c8c8c8c8c8c8c94ccc28c04cdc4982319b803370401c012600e06c66e092004009153330a301008153330a301006153330a301005153330a301004153330a301002100114a029405280a5014a02940ccccccc10cc8ccc004005200001822253330a80100114bd70099854809ba83253330a6013370e00608a2600c66e0ccdc019b823370600201a018601407601820026eb4c2a804004ccc00c00cc154008c2ac040040b80fc064060c0080fc104ccccccc1080580800f806005cc0040f8100dc100399b8700833700018014a66613c0266e1ccc1000488c26c04004cdc001da4006266e1c03c03452819b8930403370266e0802800cc004cdc0a4181341806466e092004003370400a66e1ccc0f404c8c26004004c208040e0cdc099b814830268300bc0b4dd6984f00984d809baa001163303c031037375a6138026132026ea800458cc0e80240d4dd6984d00984b809baa001163303800c033375a613002612a026ea800458cc0d80180c4dd6984b009849809baa0011633034003030375a6128026122026ea800458cc0c80080b8cc0d8020024cc0d4014018cc0cc010090cdc11bad308f010210013303b02602633031008015330300020073302f0020133302f02004f3302e01f0643370266607809a01a0180026660760c40180166eb4c21804c21c04008dd61842808009842808011bae30830100130830100237586102020026102020046eb4c1fc004c1fc004c1f8c1f8c1f8c1f8c1f8c1f8c1f8c1e8dd501f9bae307c307d002375c60f600260ee6ea8c1e8034dd6983c983d0011bac30780013078002375c60ec00260ec0046eb0c1d0004c1d0008dd6983900098390011bad30700013070001306f306f001306e002375860d800260d80046eb0c1a8004c1a8008dd69834000983418321baa05f22222223232323232323232533306e306a306f3754002264646464a6660e466e20020ccccc07c03c048028dd6983980119b8100b0061337100100022940ccccc078038044024dd6983980099b8000a005337606ea0cdc1180380099b81002005375066e08c01c004cdc000100299b83009001375a60e660e06ea800458cc044038020cdc100080499b8200a0073333301700700a0023001004003370400e66034010603000e66e08010c048020c04801c88ccc050009200022533306230030021301200110012533305e337100029000099b81480000044004c0040048894ccc1840085300103d87a8000132325333060305c0031304833064375000497ae0133300500500130470033065003375a60c600444646600200200644a6660c2002297ae0133062375060066eb4c18c004cc008008c19000488888c8c8c8c94ccc188cdc4001199998088040031824802802001899b8800200114a06666602000e00a60220080060046666601e00c00800600400266e0800cc028018cdc10019805002911998040010009119b8200200122333007002001223370200400244646600200200644a6660ba002297ae013305e3750646660280086eb8c180004dd718301830800982e1baa305f00133002002306000123330070014800088cdc00010009199803191980080080111299982d0008a5eb804c8c94ccc164c148c168dd500089980200200109982e982f182d9baa0013300400400232533305933305930550014a09444c104cc174dd4000a5eb80530103d87a8000375a60ba00460ba00290011119b82002001222223232533305a33710002004266e040080044cdc080080119b800020053370066e08014010cdc180180111119199800800802001911299982d8010a5eb804c8c94ccc17400c52f5c02660bc6ea0cc018008dd6982f8019998028028009830001982f8019bad305d002222223232533305833710002004266e040080044cdc080080119b800020043370066e0801000ccdc19980400198030028011b8048008888ccc01800c00888cc00c004008c0040048894ccc140cdc4000a4000290000a99982818260008a40042a646660a2609a66e1800920041333004004300100333706004900209800999802002180080199b83303800248010dc100111119199800800802001911299982a80108008999801801982c001198021bad3057002001375a60a20026eb4c144c148004c134dd501a0992999826182418269baa0011323232533304f330023756600660a26ea8104dd7182a18289baa004100114a0660026eacc008c140dd50200261119191980080080211299982a8008a5013253330533375e00860a860b000429444cc00c00c004c160004c0dccc14ccdd2a4004660a66ea40052f5c097ae0230523053305330533053305330530011633323001001222533305100214c0103d87a8000132325333050304c0031303833054375200497ae0133300500500130370033055003375c60a600403a900019baf374e0306e9cc0640514ccc124cdd781318270070a99982499b8702400c153330493375e6e9c088dd38050a99982499baf374e0406e9c02054ccc124cdd780f0030a99982480e08028999824802a504a229405280a5014a02940c104ccc004048dd718268011bae304d304e00222232533304b3044304c37540022900009bad3050304d375400264a666096608860986ea8004530103d87a800013233001001375660a2609c6ea8008894ccc140004530103d87a8000132323253330503371e00e6eb8c14400c4c0e0cc150dd4000a5eb804cc014014008dd69828801182a001182900099198008008021129998278008a6103d87a80001323232533304f3371e00e6eb8c14000c4c0dccc14cdd3000a5eb804cc014014008dd598280011829801182880098241baa0233375e04801a66e2120003045375460926094004609000260900046eb0c118004c118008dd6182200098220011bad30420013042001303d37540046064002607c60766ea800858c0f4c0e8dd5181e802181e181e8011bab303b001303b001303637546072606c6ea800458cc060084074c0040488c008004c004004894ccc0d000452f5c026606a6064606c00266004004606e0026eb0c0ccc0d0c0d0c0d0c0d0008cdc424000605c6ea8c0c8004c0c8008c0c0004c0c0008dd6181700098170011bac302c001302c002375a60540026054004605000260486ea807cc098c08cdd518130011bab30253026001302137546048604a0046046002603e6ea8c088c07cdd50008b198008059bad30210083001001222533302000214c0103d87a800013232533301f301b00313007330230024bd7009998028028009803001981200198110011b8048004dd2a40006038603a0046eb4c06c004c06c004c058dd5005180c180a9baa004163758602e603060300046eb0c058004c048dd5001180a180a801180980098079baa00114984d9594ccc02cc01cc030dd50008991919191919299980a180b80109924c64a666024601c002264646464a66603260380042930b1bad301a001301a002375a603000260286ea800854ccc048c02c00454ccc054c050dd50010a4c2c2c60246ea800458c054004c054008dd6980980098098011bad3011001300d37540022c600200c4a666012600a60146ea80044c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c94ccc098c0a40084c8c8c8c8c8c926330220082375a0026604201646eb8004c084050cc07c0548dd68009980f00b11810000980f00c8b1bad302700130270023758604a002604a0046eb8c08c004c08c008dd6181080098108011bad301f001301f002375a603a002603a0046eb4c06c004c06c008c94ccc060c05c00454ccc054c038c0580045288a99980a9808980b0008a501616375460320026032004602e002602e0046eb0c054004c054008dd6180980098098011bad30110013011002300f001300b37540022c6e1d200222323300100100322533300d00114984c8cc00c00cc044008c00cc03c00494ccc018c008c01cdd5000899191919299980698080010a4c2c6eb8c038004c038008dd7180600098041baa00116370e90001bae0015734aae7555cf2ab9f5740ae855d11",
        ),
        [daoVotingWitness],
        { "dataType": "list", "items": [{ "dataType": "bytes" }] },
      ),
    };
  },
  {
    inputDatum: {
      "title": "PoolData",
      "description": "Pool data.",
      "anyOf": [{
        "title": "PoolData",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          {
            "title": "poolNft",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "integer", "title": "n" },
          {
            "dataType": "list",
            "items": {
              "title": "Asset",
              "anyOf": [{
                "title": "Asset",
                "dataType": "constructor",
                "index": 0,
                "fields": [{ "dataType": "bytes", "title": "policy" }, {
                  "dataType": "bytes",
                  "title": "name",
                }],
              }],
            },
            "title": "tradableAssets",
          },
          {
            "dataType": "list",
            "items": { "dataType": "integer" },
            "title": "tradableTokensMultipliers",
          },
          {
            "title": "lpToken",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          {
            "title": "lpFeeIsEditable",
            "anyOf": [{
              "title": "False",
              "dataType": "constructor",
              "index": 0,
              "fields": [],
            }, {
              "title": "True",
              "dataType": "constructor",
              "index": 1,
              "fields": [],
            }],
          },
          { "dataType": "integer", "title": "amplCoeff" },
          { "dataType": "integer", "title": "lpFeeNum" },
          { "dataType": "integer", "title": "protocolFeeNum" },
          {
            "dataType": "list",
            "items": { "dataType": "bytes" },
            "title": "daoStabeProxyWitness",
          },
          { "dataType": "bytes", "title": "treasuryAddress" },
          {
            "dataType": "list",
            "items": { "dataType": "integer" },
            "title": "protocolFees",
          },
          { "dataType": "integer", "title": "inv" },
        ],
      }],
    },
  },
  {
    redeemer: {
      "title": "PoolRedeemer",
      "anyOf": [{
        "title": "PoolRedeemer",
        "dataType": "constructor",
        "index": 0,
        "fields": [{ "dataType": "integer", "title": "poolInIx" }, {
          "dataType": "integer",
          "title": "poolOutIx",
        }, {
          "title": "action",
          "description": "Pool action types.",
          "anyOf": [{
            "title": "AMMAction",
            "dataType": "constructor",
            "index": 0,
            "fields": [{ "dataType": "integer", "title": "optionInt0" }, {
              "dataType": "integer",
              "title": "optionInt1",
            }],
          }, {
            "title": "PDAOAction",
            "dataType": "constructor",
            "index": 1,
            "fields": [],
          }],
        }],
      }],
    },
  },
) as unknown as IPoolValidatePool;

export interface IProxyDaoStablePoolDao {
  new (): Validator;
  datum: { poolNft: { policy: string; name: string } };
  action: {
    poolInIx: bigint;
    poolOutIx: bigint;
    daoInIx: bigint;
    daoOutIx: bigint;
    daoActionIx: bigint;
  };
}

export const ProxyDaoStablePoolDao = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "5908a3010000323232323232322323232322322533300932323253323300d3001300e3754004264646464646464646464646464646464a66603a6036603c6ea80044c8c8c8c8c8c94cc8cc090c004c094dd5001099192999813181218139baa00113232323232533302b3029302c375400226464646464a666060605c60626ea80044c8c8c8c8c94ccc0d4c048c0d8dd500089919299981b981a981c1baa0011323232533303a3038303b375400226464646464a66607e603860806ea80044c8c8c8c8c8c94ccc114cdd781c182518239baa01a153330450041533304500315333045002100114a029405280a50323232323232323232323232323232323232323232323232323232323253330613370e6eb4c198c19c044dd6983318338020a99983080108008a5014a0a64646464646464646660d060cc0a026464a6660d40422a6660d40042002294052829998348038a9998348030a9998348040a9998348028a9998348020a99983480108018a5014a029405280a5014a0a6660d06010020266e2404120c0b80214a02a646660d260ba0a2264a6660d4a6660d46014020266e2404120be9a0c14a0200229414ccc1a401c54ccc1a401854ccc1a402054ccc1a401454ccc1a400454ccc1a400c40085280a5014a029405280a50153330693046051153330690071533306900615333069008153330690051533306900115333069004100214a029405280a5014a0294054ccc1a4cdc3828a400c26464a6660d6a6660d60142a6660d600e2a6660d60062a6660d600c2a6660d600a200829405280a5014a02940400452819baf374e660486604a00206e6604a0020926e9ccc090030064dd618371837983798359baa045153330693370e0a290040a9998348038a9998348030a9998348040a9998348028a9998348008a99983480208018a5014a029405280a5014a02a6660d266e1c145200a132533306a533306a300a01413371202890504e008a50100114a0a6660d200e2a6660d200c2a6660d20102a6660d20022a6660d20082a6660d2006200429405280a5014a029405280a99983499b870514803054ccc1a401c54ccc1a401854ccc1a401454ccc1a400454ccc1a401054ccc1a400c40085280a5014a029405280a5014a066e1c040070cdd79ba7375860d80186e9cdd6183600c99b8f375c60d60146eb8c1ac05ccdc38059bad306a0183370e6eb4c1a4068038cdd79ba6042374c06066ebcdd38089ba70043375e0760546e2520023370e66603607c0180166660360580180166eb0c18c004c18c004c188004c184008dd6982f800982f8011bad305d001305d002375a60b600260b660b660b660b660b660b660ae6ea8080dd7182c982d0011bae30580013054375460ae0166eb0c158004c158004c154004c150004c14c008dd698288009828800982800119b8848000c128dd518270009827000982698269826982698249baa0232232333001001003002222533304e00214bd700991929998280018a5eb804cc144dd419b81002375a60a400666600a00a00260a600660a40066eb4c14000888c8cc00400400c894ccc13000452f5c026609a6ea0c8ccc018010dd718278009bae304f3050001304b3754609c00266004004609e00244464a666090607860926ea8004520001375a609a60946ea8004c94ccc120c0f0c124dd50008a6103d87a8000132330010013756609c60966ea8008894ccc134004530103d87a80001323232533304d3371e00e6eb8c13800c4c0d0cc144dd4000a5eb804cc014014008dd698270011828801182780099198008008021129998260008a6103d87a80001323232533304c3371e00e6eb8c13400c4c0cccc140dd3000a5eb804cc014014008dd598268011828001182700099baf374c02a6e98018cdd782080199baf014006303c0013044304137540022c608660880046eacc108004c108008c100004c0f0dd5181f981e1baa001163301e0290223010003303c303937540022c603260706ea8014c0e8c0dcdd50008b181c981d0011bab30380013038002303600130323754606a60646ea800458cc05007c070dd59819981a001181900098171baa300f302e37540026060605a6ea800458cc03c06c054c00401494ccc0a4c09cc0a8dd50008991919191919191919191919191919191919191919191919191929998231824801099191919191924c6604201046eb4004cc08002c8dd7000982180a1980f00a91bad0013301d01623042001304001916375a608e002608e0046eb0c114004c114008dd7182180098218011bac30410013041002375a607e002607e0046eb4c0f4004c0f4008dd6981d800981d80119299981c181b8008a99981a9814981b0008a51153330353033303600114a02c2c6ea8c0e4004c0e4008c0dc004c0dc008dd6181a800981a8011bac30330013033002375a60620026062004605e00260566ea80045888c8cc00400400c894ccc0b800452613233003003303200230033030001302b302837540022c6010604e6ea8018c0a4c098dd50011b874801058c09cc0a0008dd598130009813001181200098101baa300130203754604660406ea80088c08cc09000458cc004034dd69810805980080091129998100010a60103d87a800013232533301f301d00313006330230024bd70099980280280099b8000348004c09000cc088008dd2a40006eb4c074c078008dd6980e000980e0011bad301a001301a002375a6030002603000260266ea802cdd6180a980b180b0011bac3014001301037540086024601e6ea8008dc3a40042c60206022004601e00260166ea8004526136565333007300530083754002264646464646464646464a666028602e0042930b1bad30150013015002375a602600260260046eb4c044004c044008dd6980780098078011bad300d001300937540022c60020084a66600a6006600c6ea80044c8c94ccc028c0340084c926300400116300b001300737540022c4a6660086004600a6ea80044c8c8c8c94ccc02cc03800852616375c601800260180046eb8c028004c018dd50008b1b87480015cd2ab9d5573caae7d5d02ba15745",
    };
  },
  {
    datum: {
      "title": "DAOData",
      "description": "DAO contract config (congig is immutable).",
      "anyOf": [{
        "title": "DAOData",
        "dataType": "constructor",
        "index": 0,
        "fields": [{
          "title": "poolNft",
          "anyOf": [{
            "title": "Asset",
            "dataType": "constructor",
            "index": 0,
            "fields": [{ "dataType": "bytes", "title": "policy" }, {
              "dataType": "bytes",
              "title": "name",
            }],
          }],
        }],
      }],
    },
  },
  {
    action: {
      "title": "DAOAction",
      "description": "DAO action types:",
      "anyOf": [{
        "title": "DAOAction",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          { "dataType": "integer", "title": "poolInIx" },
          { "dataType": "integer", "title": "poolOutIx" },
          { "dataType": "integer", "title": "daoInIx" },
          { "dataType": "integer", "title": "daoOutIx" },
          { "dataType": "integer", "title": "daoActionIx" },
        ],
      }],
    },
  },
) as unknown as IProxyDaoStablePoolDao;

export interface IRedeemRedeem {
  new (): Validator;
  datum: {
    poolNft: { policy: string; name: string };
    redeemer: string;
    expectedAssets: Array<{ policy: string; name: string }>;
    minExpectedReceivedAssetsBalances: Array<bigint>;
    minExpectedLpChange: bigint;
  };
  action: {
    ApplyOrder: {
      redeemerInIx: bigint;
      redeemerOutIx: bigint;
      poolInIx: bigint;
    };
  } | "CancelOrder";
}

export const RedeemRedeem = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "5905b401000032323232323232232323232232322533300a323232533300d300b300e375400e26464646464646464646464a666030602c60326ea80044c8c8c8c8c8c8c8c8c8c8c94ccc08cc084c090dd5000899191919299981399b8748010c0a0dd50008992999814181318149baa0011323232323232323232323232323232323232323232323232323253330453048002132323232323232323232533304c304a304d3754002264646464646464a6660a6609866600805c0660642a6660a60062a6660a6004200229405280a5032325333054305230553754002266e3c120dd7182c982b1baa00113375e600460ac6ea8c0c4c158dd501e0039800982a9baa006230580013370e66600400601000e066646600200264666002002646600200207044a6660ae002297ae013305837506466600c00e6eb8c168004dd7182d182d800982b1baa305900133002002305a001035222533305700214bd7009919299982c8018a5eb804cc168ccc158cdc49bad305b0030024c0103d87a80004c0103d8798000333005005001305c003305b003375a60b200444a6660aa00229444c94ccc14d4ccc14ccdc42400060a86ea8c1600085288999829a514a09444cc00c00c004528182c0009111929998299826182a1baa0011480004dd6982c182a9baa001325333053304c305437540022980103d87a800013233001001375660b260ac6ea8008894ccc160004530103d87a8000132323253330583371e00e6eb8c16400c4c110cc170dd4000a5eb804cc014014008dd6982c801182e001182d000991980080080211299982b8008a6103d87a8000132323253330573371e00e6eb8c16000c4c10ccc16cdd3000a5eb804cc014014008dd5982c001182d801182c8009bab305330540023052001304e375460a2609c6ea800458cc0d40e00f0dd7182798280011bae304e001304a375402e6608601046eb4004cc10802c8dd7000982100a1982000a91bad0013303f01623041001303f01916375a608c002608c0046eb0c110004c110008dd7182100098210011bac30400013040002375a607c002607c0046eb4c0f0004c0f0008dd6981d000981d00119299981b981b0008a99981a1816981a8008a51153330343032303500114a02c2c6ea8c0e0004c0e0008c0d8004c0d8008dd6181a000981a0011bac30320013032002375a60600026060004605c00260546ea800458c0b0c0a4dd50008b181598160011bab302a001302a302637546002604c6ea8c0a4c098dd50011181498150008b198060080091bae30263027002375c604a00260426ea8c090018dd6981198120011bac30220013022002375860400026040604000260366ea8060c074c068dd50008b198008029bad301c00a3001001222533301b00214c0103d87a800013232533301a3018003130063301e0024bd70099980280280099b8000348004c07c00cc074008dd2a40006eb0c060c064c064008dd6180b80098099baa007375a602a602c0046eb4c050004c050004c03cdd5003899198008008019129998090008a5013253330103371e6eb8c054008010528899801801800980a8009bae30113012300e37540166eb0c040c044c044c044c044c044c044c044c044c034dd5000980798061baa00114984d958c94ccc024c01c0044c8c8c8c8c8c94ccc048c05400852616375a602600260260046eb4c044004c044008dd6980780098059baa0031533300930020011533300c300b37540062930b0b18049baa002370e90012999802980198031baa004132323232323232323232533301230150021323232498cc0340148dd6800998060031180700098060048b1bad301300130130023758602200260220046eb0c03c004c03c008dd718068009806801180580098039baa0041622323300100100322533300b00114984c8cc00c00cc03c008c00cc03400494ccc010c008c014dd5000899191919299980598070010a4c2c6eb8c030004c030008dd7180500098031baa00116370e90002b9a5573aaae7955cfaba05742ae881",
    };
  },
  {
    datum: {
      "title": "RedeemData",
      "anyOf": [{
        "title": "RedeemData",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          {
            "title": "poolNft",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "bytes", "title": "redeemer" },
          {
            "dataType": "list",
            "items": {
              "title": "Asset",
              "anyOf": [{
                "title": "Asset",
                "dataType": "constructor",
                "index": 0,
                "fields": [{ "dataType": "bytes", "title": "policy" }, {
                  "dataType": "bytes",
                  "title": "name",
                }],
              }],
            },
            "title": "expectedAssets",
          },
          {
            "dataType": "list",
            "items": { "dataType": "integer" },
            "title": "minExpectedReceivedAssetsBalances",
          },
          { "dataType": "integer", "title": "minExpectedLpChange" },
        ],
      }],
    },
  },
  {
    action: {
      "title": "OrderAction",
      "description": "Order action types.",
      "anyOf": [{
        "title": "ApplyOrder",
        "dataType": "constructor",
        "index": 0,
        "fields": [{ "dataType": "integer", "title": "redeemerInIx" }, {
          "dataType": "integer",
          "title": "redeemerOutIx",
        }, { "dataType": "integer", "title": "poolInIx" }],
      }, {
        "title": "CancelOrder",
        "dataType": "constructor",
        "index": 1,
        "fields": [],
      }],
    },
  },
) as unknown as IRedeemRedeem;

export interface IRedeemUniformRedeemUniform {
  new (): Validator;
  datum: {
    poolNft: { policy: string; name: string };
    redeemer: string;
    minExpectedReceivedAssetsBalances: Array<bigint>;
  };
  action: {
    ApplyOrder: {
      redeemerInIx: bigint;
      redeemerOutIx: bigint;
      poolInIx: bigint;
    };
  } | "CancelOrder";
}

export const RedeemUniformRedeemUniform = Object.assign(
  function () {
    return {
      type: "PlutusV2",
      script:
        "59056601000032323232323232232323232232322533300a323232533300d300b300e375400e26464646464646464646464a666030602c60326ea80044c8c8c8c8c8c8c94ccc07cc074c080dd5000899191919299981199b8748010c090dd50008992999812181118129baa001132323232323232323232323232323232323232323232323232325333041304400213232323232323253330453043304637540022646464646464a666096608866600605405e05c2a6660960042002294052819192999826182518269baa00113371e6eb8c0b4c138dd50259bae3051304e3754002266ebcc008c138dd5181698271baa0340053001304d37546002609a6ea80188c140004c8cc004004c8ccc004004c8c8cc004004090894ccc14400452f5c02660a46ea0c8ccc01c010dd7182a0009bae305430550013050375460a60026600400460a80026eacc140c1440140c48894ccc14000852f5c026464a6660a4006297ae013305333304f337126eb4c15000c00930103d87a80004c0103d879800033300500500130550033054003375a60a400444a66609c00229444c94ccc1314ccc130cdc424000609a6ea8c14400852889998262514a09444cc00c00c0045281828800911192999826182298269baa0011480004dd6982898271baa00132533304c3045304d37540022980103d87a800013233001001375660a4609e6ea8008894ccc144004530103d87a8000132323253330513371e00e6eb8c14800c4c0f4cc154dd4000a5eb804cc014014008dd69829001182a801182980099198008008021129998280008a6103d87a8000132323253330503371e00e6eb8c14400c4c0f0cc150dd3000a5eb804cc014014008dd59828801182a0011829000982600098241baa001304a304737540022c6605c06206a6607e01046eb4004cc0f802c8dd7000981f00a1981e00a91bad0013303b0162303d001303b01916375a608400260840046eb0c100004c100008dd7181f000981f0011bac303c001303c002375a607400260740046eb4c0e0004c0e0008dd6981b000981b00119299981998190008a999818181498188008a5115333030302e303100114a02c2c6ea8c0d0004c0d0008c0c8004c0c8008dd6181800098180011bac302e001302e002375a605800260580046054002604c6ea800458c0a0c094dd50008b181398140011bab3026001302630223754600260446ea8c094c088dd50011181298130008b198040060071bae30223023002375c6042002603a6ea8c080008dd6180f98101810000980d9baa018301d301a37540022c6600200a6eb4c070028c0040048894ccc06c008530103d87a800013232533301a3018003130063301e0024bd70099980280280099b8000348004c07c00cc074008dd2a40006eb0c060c064c064008dd6180b80098099baa007375a602a602c0046eb4c050004c050004c03cdd5003899198008008019129998090008a5013253330103371e6eb8c054008010528899801801800980a8009bae30113012300e37540166eb0c040c044c044c044c044c044c044c044c044c034dd5000980798061baa00114984d958c94ccc024c01c0044c8c8c8c8c8c94ccc048c05400852616375a602600260260046eb4c044004c044008dd6980780098059baa0031533300930020011533300c300b37540062930b0b18049baa002370e90012999802980198031baa0041323232323232533300e301100213232498cc0200088dd680098040028b1bac300f001300f002375c601a002601a0046016002600e6ea80105888c8cc00400400c894ccc02c00452613233003003300f0023003300d00125333004300230053754002264646464a666016601c0042930b1bae300c001300c002375c6014002600c6ea800458dc3a4000ae6955ceaab9e5573eae815d0aba21",
    };
  },
  {
    datum: {
      "title": "RedeemUniformData",
      "anyOf": [{
        "title": "RedeemUniformData",
        "dataType": "constructor",
        "index": 0,
        "fields": [
          {
            "title": "poolNft",
            "anyOf": [{
              "title": "Asset",
              "dataType": "constructor",
              "index": 0,
              "fields": [{ "dataType": "bytes", "title": "policy" }, {
                "dataType": "bytes",
                "title": "name",
              }],
            }],
          },
          { "dataType": "bytes", "title": "redeemer" },
          {
            "dataType": "list",
            "items": { "dataType": "integer" },
            "title": "minExpectedReceivedAssetsBalances",
          },
        ],
      }],
    },
  },
  {
    action: {
      "title": "OrderAction",
      "description": "Order action types.",
      "anyOf": [{
        "title": "ApplyOrder",
        "dataType": "constructor",
        "index": 0,
        "fields": [{ "dataType": "integer", "title": "redeemerInIx" }, {
          "dataType": "integer",
          "title": "redeemerOutIx",
        }, { "dataType": "integer", "title": "poolInIx" }],
      }, {
        "title": "CancelOrder",
        "dataType": "constructor",
        "index": 1,
        "fields": [],
      }],
    },
  },
) as unknown as IRedeemUniformRedeemUniform;
