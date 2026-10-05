import fs from 'node:fs';
import { createHash } from 'node:crypto';

// CDP reports the binary that actually runs, rather than the package's pin.
export async function executedChromium(browser) {
  const session = await browser.newBrowserCDPSession();
  try {
    const version = await session.send('Browser.getVersion');
    const command = await session.send('Browser.getBrowserCommandLine');
    if (!command.arguments?.[0]) throw new Error('Chromium executable missing from CDP');
    const executable = fs.realpathSync(command.arguments[0]);
    const hash = createHash('sha256');
    for await (const chunk of fs.createReadStream(executable)) hash.update(chunk);
    return { executable, sha256: hash.digest('hex'), product: version.product, revision: version.revision, user_agent: version.userAgent, js_version: version.jsVersion };
  } finally { await session.detach(); }
}
