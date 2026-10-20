import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { mobileApi } from "../api/client";
import { completePairing } from "../pairing/pairing";
import { readPairingTicket, stripPairingTicket } from "../pairing/ticket";
import { messages } from "../i18n";

type Pending={id:string;secret:string};
export function PairPage({onPaired}:{onPaired:()=>Promise<unknown>}){const m=messages();const navigate=useNavigate();const [deviceName,setDeviceName]=useState(()=>navigator.userAgent.includes("Mobile")?"我的手机":"浏览器");const [ticket,setTicket]=useState(()=>readPairingTicket(location.hash));const [pending,setPending]=useState<Pending|null>(null);const [error,setError]=useState("");
  useEffect(()=>{if(ticket)history.replaceState(null,"",stripPairingTicket(new URL(location.href)))},[ticket]);
  useEffect(()=>{if(!pending)return;let stopped=false;const poll=async()=>{try{const result=await completePairing(pending);if(result.paired&&!stopped){await onPaired();navigate("/",{replace:true});return}}catch(cause){if(!stopped)setError(cause instanceof Error?cause.message:String(cause))}if(!stopped)setTimeout(poll,1500)};void poll();return()=>{stopped=true}},[pending,navigate,onPaired]);
  const redeem=async()=>{if(!ticket)return;try{setError("");setPending(await mobileApi.redeem(ticket,deviceName.trim()||"手机"));setTicket("")}catch(cause){setError(cause instanceof Error?cause.message:String(cause))}};
  return <main className="center"><section className="pair-card"><div className="logo">B</div><h1>{m.pair}</h1>{pending?<p>{m.waiting}</p>:<><label>{m.device}<input value={deviceName} onChange={(event)=>setDeviceName(event.target.value)}/></label><button className="primary" disabled={!ticket} onClick={()=>void redeem()}>{m.pair}</button></>}{error&&<p className="error-text">{error}</p>}</section></main>}
