import { snapshotReducer } from "./snapshot";
import type { SessionSnapshot } from "../api/types";

const snapshot = (epoch: string, seq: number): SessionSnapshot => ({
  id: "session", title: "Test", workspaces: [], model: "model", updated_at_ms: 1,
  status: "running", controllable: true, seq, epoch, target_ref: "target",
  messages: [], questions: [], synchronized: true,
});

test("忽略重复和旧序号快照", () => {
  const current = snapshot("epoch-a", 3);
  expect(snapshotReducer(current, snapshot("epoch-a", 3))).toBe(current);
  expect(snapshotReducer(current, snapshot("epoch-a", 2))).toBe(current);
});

test("新 epoch 重置序号水位", () => {
  const next = snapshot("epoch-b", 1);
  expect(snapshotReducer(snapshot("epoch-a", 9), next)).toEqual(next);
});
