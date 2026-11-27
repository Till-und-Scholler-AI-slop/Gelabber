/* global console, process */
// What a script does when the machine lacks something it needs: a program,
// a service or a variable (GEL-375). On a developer's machine it says that
// it skipped and why; in CI (`CI` set, as GitHub Actions sets it) it throws,
// so a missing package cannot turn a run green.
//
//   if (!process.env.GELABBER_SMOKE_MEDIA_PID) missing("GELABBER_SMOKE_MEDIA_PID");

/** Call where `what` is missing, then leave that part out. Throws in CI. */
export function missing(what, { ci = inCi() } = {}) {
  if (ci) throw new Error(`${what} is missing; CI must provide it (GEL-375)`);
  console.log(
    `Skipped: ${what} is missing on this machine (with CI set, this fails).`,
  );
}

/** `CI` is set to anything but the empty string. */
export function inCi() {
  return Boolean(process.env.CI);
}
