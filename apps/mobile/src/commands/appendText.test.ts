import { appendTextError } from "./appendText";

test("追加文本不得超过 16 KiB", () => {
  expect(appendTextError("a".repeat(16 * 1024))).toBeNull();
  expect(appendTextError("你".repeat(6000))).toBe("追加内容不能超过 16 KiB");
});
