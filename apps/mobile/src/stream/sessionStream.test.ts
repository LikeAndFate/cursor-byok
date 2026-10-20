import { createSessionStream } from "./sessionStream";

class FakeEventSource {
  static instances: FakeEventSource[] = [];
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  listeners = new Map<string, (event: MessageEvent) => void>();
  constructor(public url: string) { FakeEventSource.instances.push(this); }
  addEventListener(name: string, listener: EventListener) { this.listeners.set(name, listener as (event: MessageEvent) => void); }
  close() { this.closed = true; }
}

test("断线立即标记未同步", () => {
  vi.useFakeTimers();
  const states: boolean[] = [];
  createSessionStream({ url: "/events", sourceFactory: (url) => new FakeEventSource(url) as unknown as EventSource, validate: async () => undefined, onConnected: (value) => states.push(value), onSnapshot: () => undefined, onReset: () => undefined, onUnauthorized: () => undefined });
  FakeEventSource.instances[0].onopen?.();
  FakeEventSource.instances[0].onerror?.();
  expect(states).toEqual([true, false]);
  vi.useRealTimers();
});

test("销毁后取消重连且旧流不会复活", async () => {
  vi.useFakeTimers();
  FakeEventSource.instances = [];
  let validations = 0;
  const stream = createSessionStream({ url: "/events", retryMs: 100, sourceFactory: (url) => new FakeEventSource(url) as unknown as EventSource, validate: async () => { validations++; }, onConnected: () => undefined, onSnapshot: () => undefined, onReset: () => undefined, onUnauthorized: () => undefined });
  FakeEventSource.instances[0].onerror?.();
  stream.dispose();
  await vi.advanceTimersByTimeAsync(200);
  expect(validations).toBe(0);
  expect(FakeEventSource.instances).toHaveLength(1);
  expect(FakeEventSource.instances[0].closed).toBe(true);
  vi.useRealTimers();
});
