import { awaitReceipt } from "./receipt";
import type { CommandReceipt } from "../api/types";

const receipt = (status: CommandReceipt["status"]): CommandReceipt => ({ command_id: "command", status, reason: null });

test("accepted 回执轮询到终态", async () => {
  const responses = [receipt("accepted"), receipt("accepted"), receipt("applied")];
  let calls = 0;
  const result = await awaitReceipt(receipt("accepted"), async () => responses[calls++], { intervalMs: 0 });
  expect(result.status).toBe("applied");
  expect(calls).toBe(3);
});

test("终态回执不再轮询", async () => {
  let calls = 0;
  const result = await awaitReceipt(receipt("rejected"), async () => {
    calls++;
    return receipt("applied");
  }, { intervalMs: 0 });
  expect(result.status).toBe("rejected");
  expect(calls).toBe(0);
});
