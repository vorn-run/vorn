// Serves the bench's https://feeds.bench/ from the loopback fixture server, keeping the URL the connector asked for.
const base = process.env.BENCH_FEEDS;
const host = "https://feeds.bench";
const real = globalThis.fetch;

globalThis.fetch = async (input, init) => {
  const asked = typeof input === "string" ? input : String(input.url ?? input);
  if (!base || !asked.startsWith(`${host}/`)) return real(input, init);
  const res = await real(base + asked.slice(host.length), init);
  Object.defineProperty(res, "url", { value: asked });
  return res;
};
