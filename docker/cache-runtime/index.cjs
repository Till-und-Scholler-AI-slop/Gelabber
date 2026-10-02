const fs = require('node:fs');
for (const name of ['ACTIONS_RUNTIME_TOKEN', 'ACTIONS_RESULTS_URL']) {
  const value = process.env[name];
  if (!value || /[\r\n]/.test(value)) throw new Error(`Missing or invalid ${name}`);
  console.log(`::add-mask::${value}`);
  fs.appendFileSync(process.env.GITHUB_ENV, `${name}=${value}\n`);
}
