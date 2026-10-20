import type { SessionSnapshot } from "../api/types";

type Timer = ReturnType<typeof setTimeout>;
type Options = {
  url: string;
  retryMs?: number;
  sourceFactory?: (url: string) => EventSource;
  validate: () => Promise<void>;
  onConnected: (connected: boolean) => void;
  onSnapshot: (snapshot: SessionSnapshot) => void;
  onReset: () => void;
  onUnauthorized: () => void;
};

export function createSessionStream(options: Options): { dispose: () => void } {
  const sourceFactory = options.sourceFactory ?? ((url) => new EventSource(url, { withCredentials: true }));
  let active = true;
  let source: EventSource | null = null;
  let retryTimer: Timer | null = null;
  const close = () => { source?.close(); source = null; };
  const reconnect = () => {
    if (!active || !navigator.onLine || document.hidden) return;
    close();
    source = sourceFactory(options.url);
    source.onopen = () => { if (active) options.onConnected(true); };
    source.addEventListener("snapshot", (event) => {
      if (!active) return;
      options.onSnapshot(JSON.parse((event as MessageEvent).data) as SessionSnapshot);
    });
    source.addEventListener("reset", () => { if (active) options.onReset(); });
    source.onerror = () => {
      if (!active) return;
      options.onConnected(false);
      close();
      if (retryTimer !== null) clearTimeout(retryTimer);
      retryTimer = setTimeout(async () => {
        retryTimer = null;
        if (!active || !navigator.onLine || document.hidden) return;
        try {
          await options.validate();
          if (active) reconnect();
        } catch (cause) {
          if (active && isUnauthorized(cause)) options.onUnauthorized();
        }
      }, options.retryMs ?? 1500);
    };
  };
  reconnect();
  return { dispose: () => { active = false; options.onConnected(false); if (retryTimer !== null) clearTimeout(retryTimer); retryTimer = null; close(); } };
}

function isUnauthorized(cause: unknown): boolean {
  const status = cause instanceof Error ? (cause as Error & { status?: number }).status : undefined;
  return status === 401 || status === 403;
}
