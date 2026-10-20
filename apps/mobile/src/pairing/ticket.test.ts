import { readPairingTicket, stripPairingTicket } from "./ticket";

test("从 hash 读取一次性票据并从地址移除", () => {
  expect(readPairingTicket("#/pair?ticket=abc123")).toBe("abc123");
  const url = new URL("https://example.test/mobile/#/pair?ticket=abc123");
  expect(stripPairingTicket(url)).toBe("https://example.test/mobile/#/pair");
  expect(url.href).not.toContain("ticket");
});

test("没有票据时返回空字符串且不改写地址", () => {
  expect(readPairingTicket("#/pair")).toBe("");
  const url = new URL("https://example.test/mobile/#/pair");
  expect(stripPairingTicket(url)).toBe("https://example.test/mobile/#/pair");
});
