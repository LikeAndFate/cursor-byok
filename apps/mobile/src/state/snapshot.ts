import type { SessionSnapshot } from "../api/types";
export function snapshotReducer(current: SessionSnapshot | null, next: SessionSnapshot): SessionSnapshot {
  if (!current || current.epoch !== next.epoch || next.seq > current.seq) return next;
  return current;
}
