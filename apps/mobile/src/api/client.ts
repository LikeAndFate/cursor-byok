import type { CommandReceipt, QuestionAnswer, SessionSnapshot, SessionSummary, StatusView } from "./types";

async function request<T>(path:string, init?:RequestInit):Promise<T> {
  const response = await fetch(`/api${path}`, { ...init, credentials:"include", cache:"no-store", headers:{ accept:"application/json", ...(init?.body ? {"content-type":"application/json"}:{}), ...init?.headers } });
  if (!response.ok) { const body = await response.json().catch(() => ({})) as {error?:string}; const error=new Error(body.error || `${response.status} ${response.statusText}`) as Error & {status:number};error.status=response.status;throw error; }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}
export const mobileApi = {
  status: () => request<StatusView>("/status"),
  sessions: () => request<{sessions:SessionSummary[]}>("/sessions"),
  snapshot: (id:string) => request<SessionSnapshot>(`/sessions/${encodeURIComponent(id)}/snapshot`),
  redeem: (ticket:string,deviceName:string) => request<{id:string;secret:string}>("/pairings/redeem", {method:"POST", body:JSON.stringify({ticket,device_name:deviceName})}),
  command: (sessionId:string, csrf:string, body:unknown) => request<CommandReceipt>(`/sessions/${encodeURIComponent(sessionId)}/commands`, {method:"POST",headers:{"x-csrf-token":csrf},body:JSON.stringify(body)}),
  receipt: (id:string) => request<CommandReceipt>(`/commands/${encodeURIComponent(id)}`),
};
export type CommandBody = {command_id:string;target_ref:string;kind:"append_text";text:string}|{command_id:string;target_ref:string;kind:"stop"}|{command_id:string;target_ref:string;kind:"answer_question";question_ref:string;answers:QuestionAnswer[]};
