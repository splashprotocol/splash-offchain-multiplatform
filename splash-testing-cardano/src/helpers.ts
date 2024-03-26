export function numHex(num: bigint) {
  if (num === 0n) {
    return '';
  }
  const hex = num.toString(16);
  return hex.length % 2 === 0 ? hex : `0${hex}`;
}

export const strigifyAll = (_: any, value: any) => (typeof value === 'bigint' ? value.toString() : value);

export function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
