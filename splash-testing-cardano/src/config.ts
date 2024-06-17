import { strigifyAll } from "./helpers.ts";

export type Config<T> = {
  validators?: T;
};

const TEMPLATE_CONFIG_PATH = "./config.template.json";
const CONFIG_PATH = "./config.json";

export async function getConfig<T extends object>(): Promise<Config<T>> {
  try {
    return JSON.parse(await Deno.readTextFile(CONFIG_PATH));
  } catch (_) {
    // TODO: recheck recommended file
    throw Error(`
      Config file not found. Generate Config file by running deploy.ts
    `);
  }
}

async function getConfigTemplate<T extends object>(): Promise<Config<T>> {
  try {
    return JSON.parse(await Deno.readTextFile(TEMPLATE_CONFIG_PATH));
  } catch (_) {
    throw Error("Config Template not found.");
  }
}

export async function generateConfigJson<T extends object>(
  validators: T,
) {
  const newConfig = await getConfigTemplate();

  newConfig.validators = validators;

  await Deno.writeTextFile(
    CONFIG_PATH,
    JSON.stringify(newConfig, strigifyAll, 2),
  );
}
