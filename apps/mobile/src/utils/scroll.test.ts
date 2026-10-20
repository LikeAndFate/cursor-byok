import { isNearBottom } from "./scroll";

test("仅当用户原本靠近底部时才自动滚动", () => {
  expect(isNearBottom({ scrollTop: 500, scrollHeight: 1000, clientHeight: 500 })).toBe(true);
  expect(isNearBottom({ scrollTop: 0, scrollHeight: 1000, clientHeight: 500 })).toBe(false);
});
