export function canWriteCommand(state: { online:boolean; connected:boolean; synchronized:boolean; controllable:boolean; targetRef:string|null }): boolean {
  return state.online && state.connected && state.synchronized && state.controllable && Boolean(state.targetRef);
}
