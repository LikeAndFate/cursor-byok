export function isNearBottom(view: {scrollTop:number;scrollHeight:number;clientHeight:number}, threshold = 80): boolean {
  return view.scrollHeight - view.clientHeight - view.scrollTop <= threshold;
}
