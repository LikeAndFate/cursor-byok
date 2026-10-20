export function readPairingTicket(hash: string): string {
  const query = hash.includes("?") ? hash.slice(hash.indexOf("?") + 1) : "";
  return new URLSearchParams(query).get("ticket") ?? "";
}
export function stripPairingTicket(url: URL): string {
  const route = url.hash.split("?")[0] || "#/pair";
  const query = url.hash.includes("?") ? url.hash.slice(url.hash.indexOf("?") + 1) : "";
  const params = new URLSearchParams(query);
  params.delete("ticket");
  url.hash = params.size ? `${route}?${params}` : route;
  return url.href;
}
