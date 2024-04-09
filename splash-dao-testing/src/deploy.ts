import {
  Data,
  fromText,
  Lucid,
  Script,
} from "https://deno.land/x/lucid@0.10.7/mod.ts";
import { getLucid } from "./lucid.ts";
import { setupWallet } from "./wallet.ts";
import {
  NftPostmintNft,
  PermManagerPermManager,
  VeFactoryMintVeCompositionToken,
} from "./../plutus.ts";
import { VotingEscrowVotingEscrow } from "../plutus.ts";
import { VotingEscrowMintGovernancePower } from "../plutus.ts";
import { GovProxyGovProxy } from "../plutus.ts";
import { VeFactoryVeFactory } from "../plutus.ts";
import { WeightingPollWpFactory } from "../plutus.ts";
import { WeightingPollMintWpAuthToken } from "../plutus.ts";
import { SmartFarmMintFarmAuthToken } from "../plutus.ts";
import { SmartFarmFarmFactory } from "../plutus.ts";
import { VotingEscrowMintWeightingPower } from "../plutus.ts";
import { InflationInflation } from "../plutus.ts";
import {
  BuiltValidators,
  DaoInput,
  DeployedValidators,
  NFTDetails,
  ScriptNames,
} from "./types.ts";
import { NFTNames } from "./types.ts";
import { sleep } from "../../splash-testing-cardano/src/helpers.ts";

const NFT_JSON_FILENAME = "nfts.json";
const BUILT_VALIDATORS_JSON_FILENAME = "validators.json";
const DEPLOYED_VALIDATORS_JSON_FILENAME = "deployedValidators.json";
const PREPROD_DEPLOYMENT_JSON_FILENAME = "preprod.deployment.json";

async function main() {
  const lucid = await getLucid();
  await setupWallet(lucid);
  // To differentiate different deployments for testing
  const postfix = "_7";
  await mintNFTs(lucid, postfix, 1n);

  const nftDetails = JSON.parse(
    await Deno.readTextFile(NFT_JSON_FILENAME),
  );

  const txs = await deployValidators(lucid);
  const builtValidators = JSON.parse(
    await Deno.readTextFile(BUILT_VALIDATORS_JSON_FILENAME),
  );

  await getDeployedValidators(
    lucid,
    builtValidators,
    txs,
  );
  const deployedValidators = JSON.parse(
    await Deno.readTextFile(DEPLOYED_VALIDATORS_JSON_FILENAME),
  );

  // SPECIFY SETTINGS HERE -------------------------------------------------------------------------
  const acceptedAssets = new Map();
  acceptedAssets.set({
    policy: "40079b8ba147fb87a00da10deff7ddd13d64daf48802bb3f82530c3e",
    name: fromText("SPLASHTest"),
  }, { num: 100n, den: 1000n });

  const daoInput: DaoInput = {
    inflation: 1n,
    votingEscrow: {
      lockedUntil: { Def: [1n] },
      owner: "",
      maxExFee: 1n,
      version: 1n,
      lastWpEpoch: 1n,
      lastGpDeadline: 1n,
    },
    farmFactory: { lastFarmId: 1n, farmSeedData: "" },
    wpFactory: { lastPollEpoch: 0n, activeFarms: [""] },
    veFactory: {
      acceptedAssets,
      legacyAcceptedAssets: new Map(),
    },
  };

  await createEntities(lucid, deployedValidators, nftDetails, daoInput);
  const preprodConfig = {
    validators: deployedValidators,
    nfts: nftDetails,
  };
  await Deno.writeTextFile(
    PREPROD_DEPLOYMENT_JSON_FILENAME,
    toJson(preprodConfig),
  );
}

async function createMultipleMintingUtxOs(
  lucid: Lucid,
  addr: string,
  firstValuePerBox: bigint,
  subsequentValuePerBox: bigint,
) {
  const tx = await lucid.newTx()
    .payToAddress(addr, { lovelace: firstValuePerBox })
    .payToAddress(addr, { lovelace: subsequentValuePerBox })
    .payToAddress(addr, { lovelace: subsequentValuePerBox })
    .payToAddress(addr, { lovelace: subsequentValuePerBox })
    .payToAddress(addr, { lovelace: subsequentValuePerBox })
    .payToAddress(addr, { lovelace: subsequentValuePerBox })
    .complete();

  const signedTx = await tx.sign().complete();

  const txHash = await signedTx.submit();
  console.log("Creating UTxOs. tx hash: " + txHash);
  await lucid.awaitTx(txHash);
  await sleep(60000);
  return txHash;
}

async function mintNFTs(
  lucid: Lucid,
  namePostfix: string,
  quantity: bigint,
) {
  const myAddr = await lucid.wallet.address();

  const multipleUTxOTxId = await createMultipleMintingUtxOs(
    lucid,
    myAddr,
    20000000n,
    3000000n,
  );

  console.log("Waiting for multiple UTxOs to be confirmed");
  await lucid.awaitTx(multipleUTxOTxId);
  await sleep(60000);

  console.log("created multiple UTxOs.");

  const utxos = (await lucid.utxosAt(myAddr)).filter((utxo) =>
    utxo.txHash === multipleUTxOTxId
  );
  const nftDetails = buildNFTDetails(lucid, multipleUTxOTxId, namePostfix);
  let txBuilder = lucid.newTx().collectFrom(utxos);

  const keys = Object.keys(nftDetails) as NFTNames[];
  for (const key of keys) {
    const { script, policyId, assetName } = nftDetails[key];
    console.log("Policy ID for " + assetName + ": " + policyId);
    const unit = policyId + fromText(assetName);
    txBuilder = txBuilder
      .mintAssets({ [unit]: quantity }, Data.void())
      //.validTo(now + 200000)
      .attachMintingPolicy(script);
    console.log("Added mint to TX for " + assetName);
  }

  // Write to JSON
  const filePath = NFT_JSON_FILENAME;

  // Write the object to a JSON file
  await Deno.writeTextFile(filePath, JSON.stringify(nftDetails));

  const tx = await txBuilder.complete();
  const signedTx = await tx.sign().complete();
  const txHash = await signedTx.submit();
  console.log("Minting NFTs. tx hash: " + txHash);
  console.log("Waiting for TX to be confirmed");
  await lucid.awaitTx(txHash);
  await sleep(60000);

  console.log("Minted NFTs.");
}

function buildNFTDetails(
  lucid: Lucid,
  multipleUTxOTxId: string,
  namePostfix: string,
): NFTDetails {
  const toMint: [Script, string][] = [];
  for (let i = 0; i < 6; i++) {
    const script = new NftPostmintNft({
      transactionId: { hash: multipleUTxOTxId },
      outputIndex: BigInt(i),
    });

    const policyId = lucid.utils.mintingPolicyToId(script);
    toMint.push([script, policyId]);
  }

  const toBuiltPolicy = (e: [Script, string], assetName: string) => {
    return {
      script: e[0],
      policyId: e[1],
      assetName: assetName + namePostfix,
    };
  };
  return {
    factory_auth: toBuiltPolicy(toMint[0], "factory_auth"),
    gt: toBuiltPolicy(toMint[1], "gt"),
    ve_factory_auth: toBuiltPolicy(toMint[2], "ve_factory_auth"),
    perm_auth: toBuiltPolicy(toMint[3], "perm_auth"),
    proposal_auth: toBuiltPolicy(toMint[4], "proposal_auth"),
    edao_msig: toBuiltPolicy(toMint[5], "edao_msig"),
  };
}

async function deployValidators(
  lucid: Lucid,
): Promise<[string, string]> {
  const nftDetails = JSON.parse(await Deno.readTextFile(NFT_JSON_FILENAME));
  const factoryAuthPolicy = nftDetails.factory_auth.policyId;
  const votingEscrowScript = new VotingEscrowVotingEscrow(factoryAuthPolicy);
  const gtPolicy = nftDetails.gt.policyId;
  const veFactoryAuthPolicy = nftDetails.ve_factory_auth.policyId;
  const proposalAuthPolicy = nftDetails.proposal_auth.policyId;
  const permManagerAuthPolicy = nftDetails.perm_auth.policyId;
  const edaoMSig = nftDetails.edao_msig.policyId;

  const governancePowerScript = new VotingEscrowMintGovernancePower(
    proposalAuthPolicy,
    gtPolicy,
  );
  const governancePowerPolicy = lucid.utils.mintingPolicyToId(
    governancePowerScript,
  );

  const govProxyScript = new GovProxyGovProxy(
    veFactoryAuthPolicy,
    proposalAuthPolicy,
    governancePowerPolicy,
    gtPolicy,
  );
  const govProxyScriptHash = lucid.utils.validatorToScriptHash(govProxyScript);

  const veCompositionScript = new VeFactoryMintVeCompositionToken(
    factoryAuthPolicy,
  );
  const veCompositionPolicy = lucid.utils.mintingPolicyToId(
    veCompositionScript,
  );

  const veScriptHash = lucid.utils.validatorToScriptHash(
    votingEscrowScript,
  );

  const veFactoryScript = new VeFactoryVeFactory(
    factoryAuthPolicy,
    veCompositionPolicy,
    gtPolicy,
    veScriptHash,
    govProxyScriptHash,
  );
  const veFactoryScriptHash = lucid.utils.mintingPolicyToId(veFactoryScript);

  const splashPolicy =
    "40079b8ba147fb87a00da10deff7ddd13d64daf48802bb3f82530c3e";

  const farmAuthScript = new SmartFarmMintFarmAuthToken(
    splashPolicy,
    factoryAuthPolicy,
  );
  const farmAuthPolicy = lucid.utils.mintingPolicyToId(farmAuthScript);
  const zerothEpochStart = 1000n;

  const wpAuthScript = new WeightingPollMintWpAuthToken(
    splashPolicy,
    farmAuthPolicy,
    factoryAuthPolicy,
    zerothEpochStart,
  );
  const wpAuthPolicy = lucid.utils.mintingPolicyToId(wpAuthScript);

  const wpFactoryScript = new WeightingPollWpFactory(
    wpAuthPolicy,
    govProxyScriptHash,
  );
  const wpFactoryScriptHash = lucid.utils.validatorToScriptHash(
    wpFactoryScript,
  );

  const farmFactoryScript = new SmartFarmFarmFactory(
    farmAuthPolicy,
    govProxyScriptHash,
  );

  const farmFactoryScriptHash = lucid.utils.validatorToScriptHash(
    farmFactoryScript,
  );

  const weightingPowerScript = new VotingEscrowMintWeightingPower(
    zerothEpochStart,
    proposalAuthPolicy,
    gtPolicy,
  );
  const weightingPowerPolicy = lucid.utils.mintingPolicyToId(
    weightingPowerScript,
  );

  const inflationScript = new InflationInflation(
    splashPolicy,
    wpAuthPolicy,
    weightingPowerPolicy,
    zerothEpochStart,
  );

  const inflationScriptHash = lucid.utils.validatorToScriptHash(
    inflationScript,
  );

  const permManagerScript = new PermManagerPermManager(
    edaoMSig,
    permManagerAuthPolicy,
  );
  const permManagerScriptHash = lucid.utils.validatorToScriptHash(
    permManagerScript,
  );

  const builtValidators: BuiltValidators = {
    inflation: {
      script: inflationScript,
      hash: inflationScriptHash,
    },
    votingEscrow: {
      script: votingEscrowScript,
      hash: veScriptHash,
    },
    farmFactory: {
      script: farmFactoryScript,
      hash: farmFactoryScriptHash,
    },
    wpFactory: {
      script: wpFactoryScript,
      hash: wpFactoryScriptHash,
    },
    veFactory: {
      script: veFactoryScript,
      hash: veFactoryScriptHash,
    },
    govProxy: {
      script: govProxyScript,
      hash: govProxyScriptHash,
    },
    permManager: {
      script: permManagerScript,
      hash: permManagerScriptHash,
    },
    mintWPAuthToken: {
      script: wpAuthScript,
      hash: wpAuthPolicy,
    },
  };

  // Write the object to a JSON file
  await Deno.writeTextFile("validators.json", JSON.stringify(builtValidators));

  const ns: Script = lucid.utils.nativeScriptFromJson({
    type: "before",
    slot: 0,
  });
  const lockScript = lucid.utils.validatorToAddress(ns);
  const tx0 = await lucid
    .newTx()
    .payToAddressWithData(
      lockScript,
      { scriptRef: builtValidators.inflation.script },
      {},
    )
    .payToAddressWithData(
      lockScript,
      { scriptRef: builtValidators.votingEscrow.script },
      {},
    )
    .payToAddressWithData(
      lockScript,
      { scriptRef: builtValidators.farmFactory.script },
      {},
    )
    .payToAddressWithData(
      lockScript,
      { scriptRef: builtValidators.wpFactory.script },
      {},
    )
    .complete();
  const signedTx0 = await tx0.sign().complete();
  const txHash0 = await signedTx0.submit();
  console.log("Deploying validators (first batch). tx hash: " + txHash0);
  console.log("Waiting for TX to be confirmed");
  await lucid.awaitTx(txHash0);
  await sleep(60000);

  const tx = await lucid
    .newTx()
    .payToAddressWithData(
      lockScript,
      { scriptRef: builtValidators.veFactory.script },
      {},
    )
    .payToAddressWithData(
      lockScript,
      { scriptRef: builtValidators.govProxy.script },
      {},
    )
    .payToAddressWithData(
      lockScript,
      { scriptRef: builtValidators.permManager.script },
      {},
    )
    .payToAddressWithData(
      lockScript,
      { scriptRef: builtValidators.mintWPAuthToken.script },
      {},
    )
    .complete();
  const signedTx = await tx.sign().complete();
  const txHash1 = await signedTx.submit();
  console.log("Deploying validators (2nd batch). tx hash: " + txHash1);
  console.log("Waiting for TX to be confirmed");
  await lucid.awaitTx(txHash1);
  await sleep(60000);

  return [txHash0, txHash1];
}

async function createEntities(
  lucid: Lucid,
  dv: DeployedValidators,
  nftDetails: NFTDetails,
  daoInput: DaoInput,
) {
  const permManagerAuthToken = nftDetails.perm_auth.policyId +
    fromText(nftDetails.perm_auth.assetName);
  const veFactoryAuthToken = nftDetails.ve_factory_auth.policyId +
    fromText(nftDetails.ve_factory_auth.assetName);

  const toAddr = (hash: string) =>
    lucid.utils.credentialToAddress(
      { hash, type: "Script" },
    );

  const qty = 1000000n;

  const tx = await lucid.newTx()
    .readFrom([
      dv.inflation.referenceUtxo,
      dv.votingEscrow.referenceUtxo,
      dv.farmFactory.referenceUtxo,
      dv.wpFactory.referenceUtxo,
      dv.veFactory.referenceUtxo,
      dv.govProxy.referenceUtxo,
      dv.permManager.referenceUtxo,
    ])
    .payToContract(
      toAddr(dv.inflation.hash),
      Data.to(daoInput.inflation, InflationInflation.epoch),
      { lovelace: qty },
    )
    .payToContract(
      toAddr(dv.votingEscrow.hash),
      Data.to(daoInput.votingEscrow, VotingEscrowVotingEscrow.state),
      { lovelace: qty, [veFactoryAuthToken]: 1n },
    )
    .payToContract(
      toAddr(dv.farmFactory.hash),
      Data.to(daoInput.farmFactory, SmartFarmFarmFactory.state),
      { lovelace: qty },
    )
    .payToContract(
      toAddr(dv.wpFactory.hash),
      Data.to(daoInput.wpFactory, WeightingPollWpFactory.state),
      { lovelace: qty },
    )
    .payToContract(
      toAddr(dv.veFactory.hash),
      Data.to(daoInput.veFactory, VeFactoryVeFactory.conf),
      { lovelace: qty },
    )
    .payToAddress(
      toAddr(dv.govProxy.hash),
      { lovelace: qty },
    )
    .payToContract(
      toAddr(dv.permManager.hash),
      Data.void(),
      { lovelace: qty, [permManagerAuthToken]: 1n },
    ).complete();

  const signedTx = await tx.sign().complete();
  const txHash = await signedTx.submit();
  console.log("Creating entities. TX hash: " + txHash);
  console.log("Waiting for TX to be confirmed");
  await lucid.awaitTx(txHash);
  await sleep(60000);

  console.log("Entities created.");
}

async function getDeployedValidators(
  lucid: Lucid,
  builtValidators: BuiltValidators,
  deployedValidatorsTxId: [string, string],
) {
  try {
    const builtValidatorsKeys = Object.keys(builtValidators) as ScriptNames[];
    const left = builtValidatorsKeys.slice(4).map((_, index) => ({
      txHash: deployedValidatorsTxId[0],
      outputIndex: index,
    }));
    const right = builtValidatorsKeys.slice(4, builtValidatorsKeys.length).map((
      _,
      index,
    ) => ({
      txHash: deployedValidatorsTxId[1],
      outputIndex: index,
    }));

    const utxosByOutRefsRequest = left.concat(right);

    const validatorsUtxos = await lucid.utxosByOutRef(utxosByOutRefsRequest);

    const deployedValidators = builtValidatorsKeys.reduce((
      acc,
      key: ScriptNames,
      index,
    ) => {
      const { script, hash } = builtValidators[key];
      const referenceUtxo = validatorsUtxos[index];

      return {
        [key]: {
          script,
          hash,
          referenceUtxo,
        },
        ...acc,
      };
    }, {} as DeployedValidators);

    console.log(deployedValidators);

    const filepath = DEPLOYED_VALIDATORS_JSON_FILENAME;
    // Write the object to a JSON file
    await Deno.writeTextFile(
      filepath,
      toJson(deployedValidators),
    );
  } catch (error) {
    console.error("Failed to get deployed validators:", error);
    throw error;
  }
}

// From: https://stackoverflow.com/a/58253280
function toJson(data) {
  if (data !== undefined) {
    return JSON.stringify(
      data,
      (_, v) => typeof v === "bigint" ? `${v}#bigint` : v,
    )
      .replace(/"(-?\d+)#bigint"/g, (_, a) => a);
  }
}

main();
