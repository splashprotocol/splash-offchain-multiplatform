import { credentialToAddress } from "@lucid-evolution/utils";

const hash = Deno.args[0];
if (!hash) {
  console.error("Usage: limit-order-address.ts <script-hash>");
  Deno.exit(1);
}

console.log(credentialToAddress("Preprod", { type: "Script", hash }));
