// Node owns the same N+1 Janus REST long-polls as before. SSE only multiplexes
// their events on the local generator; it does not change backend sessions.
export class JanusBroker {
  constructor(poll, { deadlineMs = 65000 } = {}) {
    this.poll = poll; this.deadlineMs = deadlineMs; this.sessions = new Map();
    this.queue = []; this.trace = []; this.maximum = 0;
  }
  record(phase, session) {
    if (this.trace.length < 4096) this.trace.push({ at: Date.now(), phase, session, active_sessions: this.sessions.size });
  }
  connect(response) {
    if (this.response || this.closed) { response.writeHead(409); response.end(); return; }
    this.response = response;
    response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-store', connection: 'keep-alive' });
    response.write(': connected\n\n');
    response.on('close', () => { if (!this.closed) this.close(); });
    for (const event of this.queue.splice(0)) this.emit(event);
  }
  emit(event) {
    if (this.closed) return;
    if (!this.response) {
      if (this.queue.length >= 1024) { this.close(); throw new Error('Janus broker event overflow'); }
      this.queue.push(event); return;
    }
    // Do not let a stalled consumer accumulate unlimited media descriptions.
    if (this.response.writableLength > 1024 * 1024) { this.close(); throw new Error('Janus event consumer stalled'); }
    this.response.write('data: ' + JSON.stringify(event) + '\n\n');
  }
  add(id) {
    if (this.closed || !Number.isSafeInteger(id) || this.sessions.has(id)) throw new Error('Invalid Janus broker session');
    const entry = { controller: new AbortController() };
    this.sessions.set(id, entry); this.maximum = Math.max(this.maximum, this.sessions.size);
    entry.task = (async () => {
      while (!entry.controller.signal.aborted) {
        try {
          this.record('poll-start', id);
          const events = await this.poll(id, AbortSignal.any([entry.controller.signal, AbortSignal.timeout(this.deadlineMs)]));
          this.record('poll-complete', id);
          if (entry.controller.signal.aborted) break;
          for (const event of Array.isArray(events) ? events : [events]) {
            if (event.janus === 'error') throw new Error('Janus backend event error');
            if (event.janus !== 'keepalive') this.emit({ session_id: id, event });
          }
        } catch {
          if (!entry.controller.signal.aborted) {
            this.record('poll-failed', id); this.emit({ session_id: id, error: 'Janus backend event poll failed' });
          }
          break;
        }
      }
    })();
  }
  async remove(id) {
    const entry = this.sessions.get(id); if (!entry) return;
    this.sessions.delete(id); entry.controller.abort(); await entry.task;
    this.record('poll-cancelled', id);
  }
  async close() {
    this.closed = true; this.queue = [];
    await Promise.all([...this.sessions.keys()].map(id => this.remove(id)));
    this.response?.end();
  }
  evidence() { return { transport: 'node-backend-longpoll/browser-SSE', maximum_backend_sessions: this.maximum, remaining_backend_sessions: this.sessions.size, trace: this.trace }; }
}
