import { useEffect, useMemo, useState } from "react";
import QRCode from "qrcode";
import { Button } from "../../shared/ui/Button";
import { Checkbox } from "../../shared/ui/Checkbox";
import { FormField, TextInput } from "../../shared/ui/FormControls";
import { Switch } from "../../shared/ui/Switch";
import { TitledCard } from "../../shared/ui/TitledCard";
import { useMessage } from "../../shared/ui/message";
import styles from "./RemoteControlSettingsCard.module.scss";

type Settings = {
  enabled: boolean;
  port: number;
  public_origin: string;
  allowed_workspaces: string[];
};
type Pairing = { id: string; device_name: string; expires_at_ms: number; confirmed: boolean };
type Device = { id: string; name: string; created_at_ms: number; last_seen_at_ms: number | null; revoked_at_ms: number | null };
type RemoteView = {
  settings: Settings;
  running: boolean;
  error: string | null;
  pairings: Pairing[];
  devices: Device[];
  projects: Array<{ path: string }>;
};

async function remoteRequest<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/__byok-api__/api/remote${path}`, {
    ...init,
    headers: init?.body ? { "content-type": "application/json", ...init.headers } : init?.headers,
  });
  if (!response.ok) {
    const body = await response.text();
    throw new Error(body || `${response.status} ${response.statusText}`);
  }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}

export function RemoteControlSettingsCard() {
  const message = useMessage();
  const [view, setView] = useState<RemoteView | null>(null);
  const [draft, setDraft] = useState<Settings>({ enabled: false, port: 43127, public_origin: "", allowed_workspaces: [] });
  const [saving, setSaving] = useState(false);
  const [pairUrl, setPairUrl] = useState("");
  const [qr, setQr] = useState("");

  const report = (cause: unknown) => message(cause instanceof Error ? cause.message : String(cause));
  const load = async (replaceDraft = true) => {
    const next = await remoteRequest<RemoteView>("");
    setView(next);
    if (replaceDraft) setDraft(next.settings);
  };
  useEffect(() => {
    void load().catch(report);
    const timer = window.setInterval(() => void load(false).catch(() => undefined), 3000);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    if (!pairUrl) { setQr(""); return; }
    void QRCode.toDataURL(pairUrl, { margin: 1, width: 192, errorCorrectionLevel: "M" }).then(setQr).catch(report);
  }, [pairUrl]);

  const changed = Boolean(view && JSON.stringify(view.settings) !== JSON.stringify(draft));
  const serveCommand = useMemo(() => `tailscale serve --bg --https=443 http://127.0.0.1:${draft.port}`, [draft.port]);
  const save = async () => {
    try {
      setSaving(true);
      const next = await remoteRequest<RemoteView>("", { method: "PUT", body: JSON.stringify(draft) });
      setView(next);
      setDraft(next.settings);
      message(t("手机远控设置已保存"));
    } catch (cause) { report(cause); } finally { setSaving(false); }
  };
  const createPairing = async () => {
    try {
      const result = await remoteRequest<{ url: string }>("/pairings", { method: "POST" });
      setPairUrl(result.url);
      await load(false);
    } catch (cause) { report(cause); }
  };
  const confirm = async (id: string) => {
    try {
      await remoteRequest(`/pairings/${encodeURIComponent(id)}/confirm`, { method: "POST" });
      await load(false);
    } catch (cause) { report(cause); }
  };
  const revoke = async (id: string) => {
    try {
      await remoteRequest(`/devices/${encodeURIComponent(id)}`, { method: "DELETE" });
      await load(false);
    } catch (cause) { report(cause); }
  };

  return <TitledCard title={t("手机远控")} action={<Button size="small" variant="primary" disabled={!changed || saving} onClick={() => void save()}>{saving ? t("保存中…") : t("保存")}</Button>}>
    <div className={styles.content}>
      <div className={styles.row}>
        <div><strong>{t("开启手机远控")}</strong><small>{t("仅通过 Tailscale Serve 的 HTTPS 私网地址访问，不要直接暴露完整桌面端口。")}</small></div>
        <Switch label={t("开启手机远控")} checked={draft.enabled} disabled={!view || saving} onChange={(enabled) => setDraft((current) => ({ ...current, enabled }))} />
      </div>
      <div className={styles.status}><strong>{t("运行状态")}</strong><span data-running={view?.running || undefined}>{view?.running ? t("正在运行") : t("未运行")}</span></div>
      <div className={styles.fields}>
        <FormField label={t("监听端口")} hint={t("默认 43127，仅绑定本机回环地址。") }><TextInput type="number" min={1} max={65535} value={draft.port} onChange={(event) => setDraft((current) => ({ ...current, port: Number(event.target.value) }))} /></FormField>
        <FormField label={t("Tailscale HTTPS 来源")} hint={t("填写 Tailscale Serve 提供的 HTTPS 地址，用于生成同源配对链接。") }><TextInput placeholder="https://device.tailnet.ts.net" value={draft.public_origin} onChange={(event) => setDraft((current) => ({ ...current, public_origin: event.target.value.trim() }))} /></FormField>
      </div>
      <section><strong>{t("授权项目")}</strong><small>{t("手机只能查看和控制手动勾选的工作区。")}</small><div className={styles.list}>{view?.projects.length ? view.projects.map(({ path }) => <Checkbox key={path} label={path} checked={draft.allowed_workspaces.includes(path)} onChange={(checked) => setDraft((current) => ({ ...current, allowed_workspaces: checked ? [...current.allowed_workspaces, path] : current.allowed_workspaces.filter((value) => value !== path) }))} />) : <span>{t("尚未发现项目")}</span>}</div></section>
      <section className={styles.command}><strong>{t("Tailscale Serve 命令")}</strong><code>{serveCommand}</code><small>{t("请自行在终端执行；应用不会自动修改 Tailscale 规则。")}</small></section>
      <section><div className={styles.sectionHeader}><strong>{t("配对手机")}</strong><Button size="small" disabled={!view?.running || !draft.public_origin} onClick={() => void createPairing()}>{t("生成配对链接")}</Button></div>{pairUrl && <div className={styles.pairing}>{qr && <img src={qr} alt={t("配对二维码")} />}<code>{pairUrl}</code><small>{t("链接中的票据五分钟有效，且必须在电脑端确认。")}</small></div>}{view?.pairings.map((item) => <div className={styles.item} key={item.id}><span>{item.device_name}</span><Button size="small" disabled={item.confirmed} onClick={() => void confirm(item.id)}>{item.confirmed ? t("已确认") : t("确认")}</Button></div>)}</section>
      <section><strong>{t("已授权设备")}</strong><div className={styles.list}>{view?.devices.filter((item) => !item.revoked_at_ms).map((item) => <div className={styles.item} key={item.id}><span>{item.name}</span><Button size="small" onClick={() => void revoke(item.id)}>{t("撤销")}</Button></div>)}{!view?.devices.some((item) => !item.revoked_at_ms) && <span>{t("暂无已授权设备")}</span>}</div></section>
      {view?.error && <div className={styles.error}>{view.error}</div>}
    </div>
  </TitledCard>;
}
