import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fixtureCodecOptions, fixtureDescription } from '../video-fixture.mjs';
import { executedChromium } from '../browser-provenance.mjs';

test('one common source policy changes only VP8 video fmtp and preserves session credentials', () => {
  const sdp = 'v=0\r\na=ice-pwd:test-password\r\nm=audio 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 opus/48000/2\r\na=fmtp:96 minptime=10\r\nm=video 9 UDP/TLS/RTP/SAVPF 96 97\r\na=rtpmap:96 VP8/90000\r\na=fmtp:96 max-fr=60;x-google-min-bitrate=300;x-google-start-bitrate=600\r\na=rtpmap:97 VP9/90000\r\na=fmtp:97 profile-id=0\r\n';
  const original = { type: 'answer', sdp };
  assert.equal(fixtureDescription(original, 4000000, false), original);
  const result = fixtureDescription(original, 4000000, true);
  assert.equal(result.type, 'answer');
  assert.match(result.sdp, /a=ice-pwd:test-password/);
  assert.match(result.sdp, /a=fmtp:96 minptime=10\r\n/);
  assert.match(result.sdp, /a=fmtp:97 profile-id=0\r\n/);
  assert.match(result.sdp, /a=fmtp:96 max-fr=60;x-google-min-bitrate=4000;x-google-start-bitrate=4000;x-google-max-bitrate=4000\r\n/);
  assert.deepEqual(fixtureCodecOptions(4000000, true), { videoGoogleMinBitrate: 4000, videoGoogleStartBitrate: 4000, videoGoogleMaxBitrate: 4000 });
  assert.throws(() => fixtureCodecOptions(4000001, true), /whole kbit/);
});

test('source hints are added when a negotiated VP8 section has no fmtp', () => {
  const description = { type: 'offer', sdp: 'v=0\nm=video 9 UDP/TLS/RTP/SAVPF 100\na=rtpmap:100 VP8/90000\na=sendrecv\n' };
  assert.match(fixtureDescription(description, 4000000, true).sdp, /a=rtpmap:100 VP8\/90000\na=fmtp:100 x-google-min-bitrate=4000;.*\na=sendrecv\n/);
});

test('browser provenance hashes the executed CDP binary rather than an installed package path', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'gelabber-browser-provenance-'));
  try {
    const executable = path.join(directory, 'actual-browser'); fs.writeFileSync(executable, 'executed fixture binary');
    let detached = false;
    const browser = { newBrowserCDPSession: async () => ({
      send: async method => method === 'Browser.getVersion' ? { product: 'HeadlessChrome/153.0.8010.12', revision: '@fixture' } : { arguments: [executable, '--enable-automation'] },
      detach: async () => { detached = true; }
    }) };
    const evidence = await executedChromium(browser);
    assert.equal(evidence.executable, executable);
    assert.equal(evidence.sha256, createHash('sha256').update('executed fixture binary').digest('hex'));
    assert.equal(evidence.product, 'HeadlessChrome/153.0.8010.12');
    assert.ok(detached);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
