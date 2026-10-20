import { completePairing } from "./pairing";

test("电脑确认后返回已配对", async () => {
  const fetcher = async () => new Response(JSON.stringify({ paired: true }), { status: 200, headers: { "content-type": "application/json" } });
  expect(await completePairing({ id: "id", secret: "secret" }, fetcher)).toEqual({ paired: true });
});
