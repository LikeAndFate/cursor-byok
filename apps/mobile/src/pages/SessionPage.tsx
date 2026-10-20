import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { mobileApi, type CommandBody } from "../api/client";
import type { CommandReceipt, QuestionView, SessionSnapshot, StatusView } from "../api/types";
import { appendTextError } from "../commands/appendText";
import { canWriteCommand } from "../commands/policy";
import { awaitReceipt } from "../commands/receipt";
import { MessageText } from "../components/MessageText";
import { messages } from "../i18n";
import { buildQuestionAnswers, selectOption, setFreeformAnswer } from "../questions/answers";
import { snapshotReducer } from "../state/snapshot";
import { createSessionStream } from "../stream/sessionStream";
import { isNearBottom } from "../utils/scroll";

type PendingCommand =
  | Omit<Extract<CommandBody, { kind: "append_text" }>, "command_id" | "target_ref">
  | Omit<Extract<CommandBody, { kind: "stop" }>, "command_id" | "target_ref">
  | Omit<Extract<CommandBody, { kind: "answer_question" }>, "command_id" | "target_ref">;

type Props = {
  id: string;
  status: StatusView;
  online: boolean;
  onStatus: (status: StatusView) => void;
  onUnauthorized: () => void;
};

export function SessionPage({ id, status, online, onStatus, onUnauthorized }: Props) {
  const m = messages();
  const navigate = useNavigate();
  const [snapshot, setSnapshot] = useState<SessionSnapshot | null>(null);
  const [connected, setConnected] = useState(false);
  const [streamCycle, setStreamCycle] = useState(0);
  const [error, setError] = useState("");
  const [draft, setDraft] = useState("");
  const [draftError, setDraftError] = useState("");
  const [busy, setBusy] = useState(false);
  const [receipt, setReceipt] = useState<CommandReceipt | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const snapshotRef = useRef<SessionSnapshot | null>(null);

  const applySnapshot = (next: SessionSnapshot) => {
    setSnapshot((current) => {
      const applied = snapshotReducer(current, next);
      snapshotRef.current = applied;
      return applied;
    });
  };
  const validateAndLoad = async () => {
    setConnected(false);
    try {
      const [nextStatus, nextSnapshot] = await Promise.all([mobileApi.status(), mobileApi.snapshot(id)]);
      onStatus(nextStatus);
      applySnapshot(nextSnapshot);
      setError("");
    } catch (cause) {
      const statusCode = (cause as Error & { status?: number }).status;
      if (statusCode === 401 || statusCode === 403) {
        onUnauthorized();
        navigate("/pair", { replace: true });
        return;
      }
      setError(cause instanceof Error ? cause.message : String(cause));
      throw cause;
    }
  };

  useEffect(() => { void validateAndLoad().catch(() => undefined); }, [id]);
  useEffect(() => {
    if (!online || !snapshot) {
      setConnected(false);
      return;
    }
    const stream = createSessionStream({
      url: `/api/sessions/${encodeURIComponent(id)}/events?after=${snapshotRef.current?.seq ?? 0}`,
      validate: validateAndLoad,
      onConnected: setConnected,
      onSnapshot: applySnapshot,
      onReset: () => { void validateAndLoad().catch(() => undefined); },
      onUnauthorized: () => {
        onUnauthorized();
        navigate("/pair", { replace: true });
      },
    });
    return stream.dispose;
  }, [id, online, snapshot?.epoch, streamCycle]);
  useEffect(() => {
    const restore = () => {
      setConnected(false);
      setStreamCycle((current) => current + 1);
      if (!document.hidden && navigator.onLine) void validateAndLoad().then(() => setStreamCycle((current) => current + 1)).catch(() => undefined);
    };
    addEventListener("online", restore);
    addEventListener("offline", restore);
    document.addEventListener("visibilitychange", restore);
    return () => {
      removeEventListener("online", restore);
      removeEventListener("offline", restore);
      document.removeEventListener("visibilitychange", restore);
    };
  }, [id]);
  useLayoutEffect(() => {
    if (follow.current && viewport.current) viewport.current.scrollTop = viewport.current.scrollHeight;
  }, [snapshot?.seq]);

  const writable = Boolean(snapshot && canWriteCommand({
    online,
    connected,
    synchronized: snapshot.synchronized,
    controllable: snapshot.controllable,
    targetRef: snapshot.target_ref,
  }));
  const send = async (body: PendingCommand): Promise<boolean> => {
    if (!snapshot?.target_ref || !writable) return false;
    setBusy(true);
    setReceipt(null);
    const command_id = crypto.randomUUID();
    try {
      const initial = await mobileApi.command(id, status.csrf_token, { ...body, command_id, target_ref: snapshot.target_ref });
      setReceipt(await awaitReceipt(initial, () => mobileApi.receipt(command_id)));
      await validateAndLoad();
      return true;
    } catch (cause) {
      if ((cause as Error & { status?: number }).status === 401) {
        onUnauthorized();
        navigate("/pair", { replace: true });
      }
      setReceipt({ command_id, status: "outcome_unknown", reason: cause instanceof Error ? cause.message : String(cause) });
      return false;
    } finally {
      setBusy(false);
    }
  };
  const append = async () => {
    const text = draft.trim();
    const validation = appendTextError(text);
    if (validation) { setDraftError(validation); return; }
    setDraftError("");
    if (await send({ kind: "append_text", text })) setDraft("");
  };

  return <main className="session-shell">
    <header className="session-header"><Link to="/">← {m.back}</Link><div><strong>{snapshot?.title || id}</strong><span className={connected ? "dot ok" : "dot"} /></div></header>
    <div className="messages" ref={viewport} onScroll={(event) => { follow.current = isNearBottom(event.currentTarget); }}>
      {error && <button className="error" onClick={() => void validateAndLoad().catch(() => undefined)}>{m.failed} · {m.retry}</button>}
      {snapshot?.messages.map((item) => <article key={item.id} className={`message ${item.kind}`}><span>{item.kind}</span><MessageText text={item.text} truncated={item.truncated} /></article>)}
      {snapshot?.questions.map((question) => <QuestionForm key={question.ref} question={question} disabled={!writable || busy} onSubmit={(answers) => void send({ kind: "answer_question", question_ref: question.ref, answers })} />)}
    </div>
    <section className="composer">
      {!online ? <small>{m.offline}</small> : !connected || (snapshot && !snapshot.synchronized) ? <small>{m.stale}</small> : !snapshot?.target_ref ? <small>{m.noTarget}</small> : null}
      <textarea value={draft} disabled={!writable || busy} placeholder={m.appendHint} onChange={(event) => { setDraft(event.target.value); setDraftError(""); }} />
      {draftError && <small className="error-text">{draftError}</small>}
      <div><button disabled={!writable || busy} onClick={() => void send({ kind: "stop" })}>{m.stop}</button><button className="primary" disabled={!writable || busy || !draft.trim()} onClick={() => void append()}>{m.send}</button></div>
      {receipt && <small className={`receipt ${receipt.status}`}>{receipt.status === "applied" ? m.receiptApplied : receipt.status === "rejected" ? m.receiptRejected : m.receiptUnknown}{receipt.reason ? `：${receipt.reason}` : ""}</small>}
    </section>
  </main>;
}

function QuestionForm({ question, disabled, onSubmit }: { question: QuestionView; disabled: boolean; onSubmit: (answers: ReturnType<typeof buildQuestionAnswers>) => void }) {
  const m = messages();
  const [selected, setSelected] = useState<Record<string, string[]>>({});
  const [freeform, setFreeform] = useState<Record<string, string>>({});
  const apply = (next: { selected: Record<string, string[]>; freeform: Record<string, string> }) => { setSelected(next.selected); setFreeform(next.freeform); };
  return <section className="question"><h2>{question.title || m.question}</h2>{question.questions.map((item) => <fieldset key={item.id}><legend>{item.prompt}</legend>{item.options.map((option) => <label key={option.id}><input type={item.allow_multiple ? "checkbox" : "radio"} name={item.id} checked={(selected[item.id] ?? []).includes(option.id)} disabled={disabled} onChange={(event) => apply(selectOption(item, selected, freeform, option.id, event.target.checked))} />{option.label}</label>)}<input placeholder={m.other} disabled={disabled} value={freeform[item.id] ?? ""} onChange={(event) => apply(setFreeformAnswer(selected, freeform, item.id, event.target.value))} /></fieldset>)}<button className="primary" disabled={disabled} onClick={() => onSubmit(buildQuestionAnswers(question, selected, freeform))}>{m.submit}</button></section>;
}
