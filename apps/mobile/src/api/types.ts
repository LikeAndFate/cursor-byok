export type SessionStatus = "running" | "waiting_question" | "completed" | "cancelled" | "failed" | "unavailable";
export interface SessionSummary { id:string; title:string; workspaces:string[]; model:string; updated_at_ms:number; status:SessionStatus; controllable:boolean }
export interface MessageView { id:string; kind:"user"|"assistant"|"tool"|"notice"; text:string; truncated:boolean }
export interface QuestionItem { id:string; prompt:string; allow_multiple:boolean; options:Array<{id:string;label:string}> }
export interface QuestionView { ref:string; title:string; questions:QuestionItem[] }
export interface SessionSnapshot extends SessionSummary { seq:number; epoch:string; target_ref:string|null; messages:MessageView[]; questions:QuestionView[]; synchronized:boolean }
export interface StatusView { device_id:string; csrf_token:string; epoch:string; host_name:string; version:string }
export interface CommandReceipt { command_id:string; status:"accepted"|"applied"|"rejected"|"outcome_unknown"; reason:string|null }
export interface QuestionAnswer { question_id:string; selected_option_ids:string[]; freeform_text:string }
