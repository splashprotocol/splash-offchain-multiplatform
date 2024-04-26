import { strigifyAll } from './helpers.ts';

export type Config<T> = {
  validators?: T
};

const TEMPLATE_CONFIG_PATH = './config.template.json';
const CONFIG_PATH = './config.json';

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
    throw Error('Config Template not found.');
  }
}

const stringifyBigIntReviewer = (_: any, value: any) =>
  typeof value === 'bigint'
    ? { value: value.toString(), _bigint: true }
    : value;

export async function generateConfigJson<T extends object>(
  validators: T,
) {

  const prevConfig = await getConfig();

  const newConfig = await getConfigTemplate();

  newConfig.validators = { ...prevConfig.validators, ...validators  };

  // console.log(`${JSON.stringify(validators, stringifyBigIntReviewer)}`)

  // console.log(`${JSON.stringify(prevConfig.validators, stringifyBigIntReviewer)}`)

  // newConfig.validators = validators ++ prevConfig.validators!;

  await Deno.writeTextFile(CONFIG_PATH, JSON.stringify(newConfig, strigifyAll, 2));
}
