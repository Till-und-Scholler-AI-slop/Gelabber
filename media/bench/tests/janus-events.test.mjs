import { test } from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { JanusSignal } from '../janus-events.mjs';
import { JanusBroker } from '../janus-broker.mjs';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const json = value => ({ ok: true, json: async () => value });
function setup(fetcher, deadlineMs = 100) {
  const stream = { close() { this.closed = true; } }, failures = [];
  const signal = new JanusSignal({ fetcher, eventSource: () => stream, failures, deadlineMs });
  stream.onopen(); return { signal, stream, failures };
}

test('eight peer events match their own transaction, including events preceding ACK', async () => {
  let signal, id = 0;
  ({ signal } = setup(async (url, options) => {
    const body = JSON.parse(options.body);
    if (body.janus === 'create') return json({ janus: 'success', data: { id: ++id } });
    signal.deliver({ session_id: Number(url.split('/')[2]), event: { transaction: body.transaction, plugindata: { data: { peer: body.body.peer } } } });
    await delay(5); return json({ janus: 'ack' });
  }));
  try {
    await signal.ready;
    const sessions = await Promise.all(Array.from({ length: 9 }, () => signal.session()));
    const results = await Promise.all(sessions.slice(1).map((session, peer) => session.request(99, { janus: 'message', body: { request: 'join', peer } })));
    assert.deepEqual(results.map(r => r.plugindata.data.peer), [0, 1, 2, 3, 4, 5, 6, 7]);
    assert.equal(signal.controllers.size, 0);
    assert.equal(sessions.reduce((n, s) => n + s.pending.size, 0), 0);
  } finally { signal.close(); }
});

test('deadline covers stalled POST even after an early matching event', async () => {
  let signal, body;
  ({ signal } = setup(async (url, options) => {
    body = JSON.parse(options.body);
    if (body.janus === 'create') return json({ janus: 'success', data: { id: 1 } });
    signal.deliver({ session_id: 1, event: { transaction: body.transaction } });
    return new Promise(() => {}); // Deliberately ignores AbortSignal.
  }, 20));
  try {
    const session = await signal.session();
    await assert.rejects(session.request(1, { janus: 'message' }), /deadline/);
    assert.equal(session.pending.size, 0); assert.equal(signal.controllers.size, 0);
    signal.deliver({ session_id: 1, event: { transaction: body.transaction } });
    assert.equal(session.pending.size, 0);
  } finally { signal.close(); }
});

test('missing event after ACK rejects and deletes pending state', async () => {
  const { signal } = setup(async (_, options) => json(JSON.parse(options.body).janus === 'create'
    ? { janus: 'success', data: { id: 1 } } : { janus: 'ack' }), 20);
  try {
    const session = await signal.session();
    await assert.rejects(session.request(1, { janus: 'message' }), /deadline/);
    assert.equal(session.pending.size, 0);
  } finally { signal.close(); }
});

test('event connection failure and explicit close abort in-flight request', async () => {
  for (const mode of ['error', 'close']) {
    const { signal, stream, failures } = setup(async (_, options) => JSON.parse(options.body).janus === 'create'
      ? json({ janus: 'success', data: { id: 1 } }) : new Promise(() => {}));
    const session = await signal.session(), pending = session.request(1, { janus: 'message' });
    if (mode === 'error') stream.onerror(); else signal.close();
    await assert.rejects(pending, /connection failed|cancelled/);
    assert.equal(session.pending.size, 0); assert.equal(signal.controllers.size, 0); assert.equal(stream.closed, true);
    assert.equal(failures.length, mode === 'error' ? 1 : 0); signal.close();
  }
});

test('broker retains N+1 backend long-polls and aborts every owned poll on close', async () => {
  const active = new Set();
  const broker = new JanusBroker((id, signal) => new Promise((_, reject) => {
    active.add(id); signal.addEventListener('abort', () => { active.delete(id); reject(signal.reason); }, { once: true });
  }));
  for (let id = 1; id <= 9; id++) broker.add(id);
  assert.equal(active.size, 9); assert.equal(broker.evidence().maximum_backend_sessions, 9);
  await broker.remove(1); assert.equal(active.size, 8);
  await broker.close(); assert.equal(active.size, 0); assert.equal(broker.evidence().remaining_backend_sessions, 0);
});

test('backend poll deadline is a visible failure, not an endless retry', async () => {
  const response = new EventEmitter(); response.writableLength = 0; response.writeHead = () => {};
  const output = []; response.write = chunk => output.push(chunk); response.end = () => {};
  const broker = new JanusBroker((_, signal) => new Promise((_, reject) => {
    signal.addEventListener('abort', () => reject(signal.reason), { once: true });
  }), { deadlineMs: 10 });
  broker.connect(response); broker.add(1); await delay(30);
  assert.match(output.join(''), /Janus backend event poll failed/);
  assert.equal(broker.trace.filter(entry => entry.phase === 'poll-start').length, 1);
  await broker.close();
});
