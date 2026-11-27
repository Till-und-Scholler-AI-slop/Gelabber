/* global console */
// Checks known to fail, each under the issue that ends it and with the failure
// it expects. A run passes when at least one check passed, every other check
// passed and every listed one failed with its message. A listed check that
// passes fails the run, so its entry is removed with the fix; one that fails
// with another message is a failure like any other.

/**
 * `expected`: check name -> { issue, message: RegExp }. `passed`: names of
 * passed checks. `failed`: [{ name, message }]. Returns what is wrong, empty
 * when the run passes, and says which known failures it saw.
 */
export function settle(expected, { passed, failed }) {
  const problems = [];
  for (const { name, message } of failed) {
    const known = expected[name];
    if (!known) problems.push(`${name}: ${message}`);
    else if (!known.message.test(message))
      problems.push(
        `${name}: ${message} (listed under ${known.issue} for ${known.message}, failed otherwise)`,
      );
    else console.log(`EXPECTED FAIL ${name} (${known.issue}): ${message}`);
  }
  for (const name of passed)
    if (expected[name])
      problems.push(
        `${name} passes: remove it from the expected failures (${expected[name].issue})`,
      );
  if (passed.length === 0) problems.push("no check passed");
  return problems;
}
