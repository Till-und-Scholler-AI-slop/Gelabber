/** Kick/ban is server-scoped. Only leave the current view when it is
 *  that server — a kick on A must not yank the user out of B. */
export function shouldLeaveView(
  viewingServerId: string | undefined,
  removedServerId: string,
): boolean {
  return viewingServerId === removedServerId;
}
