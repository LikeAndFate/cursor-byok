import type { QuestionAnswer, QuestionItem, QuestionView } from "../api/types";
export type AnswerState = { selected: Record<string,string[]>; freeform: Record<string,string> };
export function buildQuestionAnswers(view: QuestionView, selected: Record<string,string[]>, freeform: Record<string,string>): QuestionAnswer[] {
  return view.questions.map((question) => ({
    question_id: question.id,
    selected_option_ids: freeform[question.id]?.trim() ? [] : question.allow_multiple ? (selected[question.id] ?? []) : (selected[question.id] ?? []).slice(0, 1),
    freeform_text: freeform[question.id]?.trim() ?? "",
  }));
}
export function setFreeformAnswer(selected: Record<string,string[]>, freeform: Record<string,string>, questionId:string, value:string):AnswerState {
  return { selected:{...selected,[questionId]:value ? [] : selected[questionId] ?? []}, freeform:{...freeform,[questionId]:value} };
}
export function selectOption(question:QuestionItem, selected:Record<string,string[]>, freeform:Record<string,string>, optionId:string, checked:boolean):AnswerState {
  const current=selected[question.id]??[];
  const next=question.allow_multiple?(checked?[...current.filter((id)=>id!==optionId),optionId]:current.filter((id)=>id!==optionId)):(checked?[optionId]:[]);
  return {selected:{...selected,[question.id]:next},freeform:{...freeform,[question.id]:""}};
}
