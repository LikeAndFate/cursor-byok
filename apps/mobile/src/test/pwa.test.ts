import { readFileSync } from "node:fs";
import { resolve } from "node:path";

test("PWA runtime cache 排除 API 和认证数据", () => {
  const config = readFileSync(resolve(process.cwd(), "vite.config.ts"), "utf8");
  expect(config).toContain("navigateFallbackDenylist");
  expect(config).toContain("/^\\/api\\//");
  expect(config).not.toContain("NetworkFirst");
});
