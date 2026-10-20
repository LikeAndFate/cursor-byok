import { canWriteCommand } from "./policy";

const ready = { online: true, connected: true, synchronized: true, controllable: true, targetRef: "target" };

test("离线、同步失效或无活动 target 时禁写", () => {
  expect(canWriteCommand({ ...ready, online: false })).toBe(false);
  expect(canWriteCommand({ ...ready, connected: false })).toBe(false);
  expect(canWriteCommand({ ...ready, synchronized: false })).toBe(false);
  expect(canWriteCommand({ ...ready, targetRef: null })).toBe(false);
  expect(canWriteCommand(ready)).toBe(true);
});
