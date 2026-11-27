/* global console, process */
// What a script does when the machine lacks something it needs: a program,
// a service or a variable (GEL-375). On a developer's machine it says that
// it skipped and why; in CI it throws, so a missing package cannot turn a
// run green. CI means the variable `CI` is set to anything but "", "0" or
// "false" (GitHub Actions sets "true").
//
//   if (!process.env.GELABBER_SMOKE_MEDIA_PID) missing("GELABBER_SMOKE_MEDIA_PID");

/** Call where `what` is missing, then leave that part out. Throws in CI. */
export function missing(what, { ci = inCi() } = {}) {
  if (ci)
    throw new Error(
      `${what} is missing; CI must provide it (GEL-375). (CI is set; unset it to skip here)`,
    );
  console.log(
    `Skipped: ${what} is missing on this machine (with CI set, this fails).`,
  );
}

/** `CI` is set, and not to "", "0" or "false" (in any case). */
export function inCi(value = process.env.CI) {
  return !["", "0", "false"].includes((value ?? "").trim().toLowerCase());
}
