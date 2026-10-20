import { buildQuestionAnswers, selectOption, setFreeformAnswer } from "./answers";
import type { QuestionView } from "../api/types";

const view: QuestionView = { ref: "question", title: "Choose", questions: [
  { id: "single", prompt: "One", allow_multiple: false, options: [{ id: "a", label: "A" }, { id: "b", label: "B" }] },
  { id: "multi", prompt: "Many", allow_multiple: true, options: [{ id: "x", label: "X" }, { id: "y", label: "Y" }] },
] };

test("问题答案保留单选、多选和其他文本", () => {
  expect(buildQuestionAnswers(view, { single: ["b", "a"], multi: ["x", "y"] }, { single: "other" })).toEqual([
    { question_id: "single", selected_option_ids: [], freeform_text: "other" },
    { question_id: "multi", selected_option_ids: ["x", "y"], freeform_text: "" },
  ]);
});

test("自由文本和选项互斥", () => {
  expect(setFreeformAnswer({ single: ["a"] }, {}, "single", "custom")).toEqual({ selected: { single: [] }, freeform: { single: "custom" } });
  expect(selectOption(view.questions[0], { single: [] }, { single: "custom" }, "b", true)).toEqual({ selected: { single: ["b"] }, freeform: { single: "" } });
});
