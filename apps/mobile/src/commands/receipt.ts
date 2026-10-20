import type { CommandReceipt } from "../api/types";
export async function awaitReceipt(initial: CommandReceipt, getReceipt: () => Promise<CommandReceipt>, options: {intervalMs?:number; maxAttempts?:number} = {}): Promise<CommandReceipt> {
  let receipt = initial;
  const interval = options.intervalMs ?? 700;
  for (let attempt = 0; receipt.status === "accepted" && attempt < (options.maxAttempts ?? 60); attempt++) {
    if (interval) await new Promise((resolve) => setTimeout(resolve, interval));
    receipt = await getReceipt();
  }
  return receipt.status === "accepted" ? { ...receipt, status:"outcome_unknown", reason:"等待命令结果超时" } : receipt;
}
