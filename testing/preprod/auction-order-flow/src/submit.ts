import { CML } from "@lucid-evolution/lucid";
import { blockfrostBaseUrl, ProviderEnv } from "./env.ts";

const SUBMIT_TIMEOUT_MS = 20_000;
const LOOKUP_TIMEOUT_MS = 10_000;

function bytesFromHex(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error("Expected even-length hex");
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

async function fetchWithTimeout(
  url: string,
  init: RequestInit,
  timeoutMs: number,
): Promise<Response> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timeout);
  }
}

export async function submitSignedTx(
  env: ProviderEnv,
  signed: { submit(): Promise<string>; toCBOR(): string },
): Promise<string> {
  if (env.provider !== "koios") return await signed.submit();
  let lastError: unknown;
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      const res = await fetchWithTimeout(
        "https://preprod.koios.rest/api/v1/submittx",
        {
          method: "POST",
          headers: {
            "content-type": "application/cbor",
            accept: "application/json",
          },
          body: bytesFromHex(signed.toCBOR()),
        },
        SUBMIT_TIMEOUT_MS,
      );
      if (!res.ok) {
        throw new Error(
          `Koios submit failed: ${res.status} ${await res.text()}`,
        );
      }
      return await res.json();
    } catch (error) {
      lastError = error;
      if (attempt < 3) {
        await new Promise((resolve) => setTimeout(resolve, 2000));
      }
    }
  }
  throw new Error(`Koios submit failed after retries: ${lastError}`);
}

export async function fetchSubmittedOutputIndex(
  env: ProviderEnv,
  txHash: string,
  address: string,
  datum: string,
): Promise<number> {
  const datumHash = CML.hash_plutus_data(CML.PlutusData.from_cbor_hex(datum))
    .to_hex();
  if (env.provider === "koios") {
    const url = "https://preprod.koios.rest/api/v1/tx_info";
    const headers = { "content-type": "application/json" };
    const body = JSON.stringify({ _tx_hashes: [txHash], _scripts: true });
    for (let attempt = 0; attempt < 60; attempt++) {
      try {
        const res = await fetchWithTimeout(
          url,
          { method: "POST", headers, body },
          LOOKUP_TIMEOUT_MS,
        );
        if (res.ok) {
          const txs = await res.json();
          const outputIndex = txs[0]?.outputs?.find((
            out: {
              tx_index: number;
              address?: string;
              payment_addr?: { bech32?: string };
              datum_hash?: string | null;
              inline_datum?: { bytes?: string } | string | null;
            },
          ) => {
            const outAddress = out.address ?? out.payment_addr?.bech32;
            const inlineDatum = typeof out.inline_datum === "string"
              ? out.inline_datum
              : out.inline_datum?.bytes;
            return outAddress === address &&
              (inlineDatum === datum || out.datum_hash === datumHash);
          })?.tx_index;
          if (outputIndex !== undefined) return outputIndex;
        }
      } catch {
        // Retry transient provider timeouts until the bounded polling window ends.
      }
      await new Promise((resolve) => setTimeout(resolve, 2000));
    }
  } else {
    const headers = { project_id: env.blockfrostProjectId! };
    const url = `${blockfrostBaseUrl(env)}/txs/${txHash}/utxos`;
    for (let attempt = 0; attempt < 30; attempt++) {
      const res = await fetch(url, { headers });
      if (res.ok) {
        const body = await res.json();
        const outputIndex = body.outputs.find((
          out: { output_index: number; address: string; inline_datum?: string },
        ) => out.address === address && out.inline_datum === datum)
          ?.output_index;
        if (outputIndex !== undefined) return outputIndex;
      }
      await new Promise((resolve) => setTimeout(resolve, 2000));
    }
  }
  console.error(
    `Submitted tx ${txHash}, but provider lookup could not locate output index; using harness default output index 0`,
  );
  return 0;
}
