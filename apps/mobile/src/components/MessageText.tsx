export function MessageText({ text, truncated }: { text:string; truncated:boolean }) {
  return <div className="message-text"><pre>{text}</pre>{truncated && <span className="truncated">内容已截断</span>}</div>;
}
