import assert from 'node:assert/strict';
import test from 'node:test';
import { proxyTarget } from '../proxy-target.mjs';
test('only backend origin receives benchmark authorization', () => {
  const backend = 'http://127.0.0.1:8091';
  assert.equal(proxyTarget('/rpc', backend).href, backend + '/rpc');
  assert.equal(proxyTarget('/janus/123?rid=4', backend).origin, backend);
  for (const foreign of ['https://foreign.invalid/rpc', '//foreign.invalid/rpc', 'http://127.0.0.1:80/rpc', '/\\foreign.invalid/rpc']) {
    assert.throws(() => proxyTarget(foreign, backend), /foreign proxy origin/);
  }
});
