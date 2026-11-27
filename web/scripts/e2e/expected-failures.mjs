/* global console */
// Checks known to fail, each under the issue that ends it. A run passes when
// every other check passed and every listed one failed; a listed check that
// passes fails the run, so its entry is removed with the fix.

/**
 * `expected`: check name -> issue. `passed`: names of passed checks.
 * `failed`: [{ name, message }]. Returns what is wrong, empty when the run
 * passes, and says which known failures it saw.
 */
export function settle(expected, { passed, failed }) {
  const problems = [];
  for (const { name, message } of failed)
    if (expected[name])
      console.log(`EXPECTED FAIL ${name} (${expected[name]}): ${message}`);
    else problems.push(`${name}: ${message}`);
  for (const name of passed)
    if (expected[name])
      problems.push(
        `${name} passes: remove it from the expected failures (${expected[name]})`,
      );
  return problems;
}
