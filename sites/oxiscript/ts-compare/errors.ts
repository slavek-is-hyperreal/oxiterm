function parsePort(n: number): number {
  if (n < 1 || n > 65535) {
    throw new Error("port out of range");
  }
  return n;
}

export function describe(n: number): string {
  try {
    parsePort(n);
    return "port ok";
  } catch (e) {
    return (e as Error).message;
  }
}
