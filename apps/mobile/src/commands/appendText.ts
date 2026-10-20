const MAX_APPEND_BYTES = 16 * 1024;
export function appendTextError(text: string): string | null {
  if (new TextEncoder().encode(text).byteLength > MAX_APPEND_BYTES) return "追加内容不能超过 16 KiB";
  return null;
}
