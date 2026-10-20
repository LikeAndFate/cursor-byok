import { useEffect, useMemo, useState } from "react";
import { HashRouter, Navigate, Route, Routes, useNavigate, useParams } from "react-router-dom";
import { mobileApi } from "./api/client";
import type { SessionSummary, StatusView } from "./api/types";
import { PairPage } from "./pages/PairPage";
import { SessionPage } from "./pages/SessionPage";
import { messages, resolveLocale } from "./i18n";

function SessionsPage({ online }: {online:boolean}) {
  const [sessions,setSessions]=useState<SessionSummary[]>([]); const [error,setError]=useState(""); const navigate=useNavigate(); const m=messages();
  const load=()=>mobileApi.sessions().then((value)=>{setSessions(value.sessions);setError("")}).catch((cause)=>setError(String(cause instanceof Error?cause.message:cause)));
  useEffect(()=>{void load()},[]);
  return <main className="shell"><header><div><h1>{m.sessions}</h1><span className={online?"badge ok":"badge"}>{online?m.online:m.offline}</span></div></header><section className="session-list">
    {error&&<button className="error" onClick={load}>{m.failed} · {m.retry}</button>}
    {!error&&!sessions.length&&<div className="empty">{m.empty}</div>}
    {sessions.map((session)=><button className="session-card" key={session.id} onClick={()=>navigate(`/sessions/${encodeURIComponent(session.id)}`)}><strong>{session.title||session.id}</strong><span>{session.model} · {session.status}</span><small>{session.workspaces.join(" · ")}</small></button>)}
  </section><footer>{m.install}</footer></main>;
}
function RoutedApp(){ const [online,setOnline]=useState(navigator.onLine); const [status,setStatus]=useState<StatusView|null>(null); const locale=useMemo(resolveLocale,[]);
  useEffect(()=>{document.documentElement.lang=locale; const on=()=>setOnline(true),off=()=>setOnline(false); addEventListener("online",on);addEventListener("offline",off); return()=>{removeEventListener("online",on);removeEventListener("offline",off)}},[locale]);
  useEffect(()=>{if(online) void mobileApi.status().then(setStatus).catch(()=>setStatus(null))},[online]);
  return <Routes><Route path="/pair" element={<PairPage onPaired={()=>mobileApi.status().then(setStatus)} />}/><Route path="/" element={status?<SessionsPage online={online}/>:<Navigate to="/pair" replace/>}/><Route path="/sessions/:id" element={status?<SessionRoute status={status} online={online} onStatus={setStatus} onUnauthorized={()=>setStatus(null)}/>:<Navigate to="/pair" replace/>}/></Routes>;
}
function SessionRoute({status,online,onStatus,onUnauthorized}:{status:StatusView;online:boolean;onStatus:(status:StatusView)=>void;onUnauthorized:()=>void}){const {id=""}=useParams();return <SessionPage id={decodeURIComponent(id)} status={status} online={online} onStatus={onStatus} onUnauthorized={onUnauthorized}/>}export function App(){return <HashRouter><RoutedApp/></HashRouter>}
