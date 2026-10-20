type Fetcher = typeof fetch;
export async function completePairing(credentials: {id:string;secret:string}, fetcher: Fetcher = fetch): Promise<{paired:boolean}> {
  const response = await fetcher("/api/pairings/complete", { method:"POST", credentials:"include", headers:{"content-type":"application/json"}, body:JSON.stringify(credentials) });
  if (response.status === 202) return { paired:false };
  if (!response.ok) throw new Error((await response.json() as {error?:string}).error ?? "配对失败");
  return { paired:(await response.json() as {paired:boolean}).paired === true };
}
