import { render, screen } from "@testing-library/react";
import { MessageText } from "./MessageText";

test("危险 HTML 只作为纯文本展示", () => {
  render(<MessageText text={'<img src=x onerror="alert(1)">'} truncated={false} />);
  expect(screen.getByText('<img src=x onerror="alert(1)">')).toBeInTheDocument();
  expect(document.querySelector("img")).toBeNull();
});
