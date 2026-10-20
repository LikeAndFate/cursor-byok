import { completePairing } from "./pairing";

test("电脑未确认时不会建立会话", async () => {
  const fetcher = async () => new Response(JSON.stringify({ pending: true }), { status: 202, headers: { "content-type": "application/json" } });
  expect(await completePairing({ id: "id", secret: "secret" }, fetcher)).toEqual({ paired: false });
});
