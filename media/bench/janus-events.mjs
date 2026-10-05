// Browser-side transaction matching. One event connection avoids Chromium's
// HTTP/1 per-origin pool being occupied by one long-poll per participant.
export class JanusSignal {
  constructor({ fetcher = (...args) => fetch(...args), eventSource = url => new EventSource(url), deadlineMs = 30000, failures = [] } = {}) {
    this.fetcher = fetcher; this.deadlineMs = deadlineMs; this.failures = failures;
    this.sessions = new Map(); this.controllers = new Set(); this.backlog = []; this.trace = [];
    this.stream = eventSource('/janus-events');
    this.ready = new Promise((resolve, reject) => {
      this.readyReject = reject;
      this.readyTimer = setTimeout(() => this.fail(new Error('Janus event connection deadline')), deadlineMs);
      this.stream.onopen = () => { clearTimeout(this.readyTimer); resolve(); };
    });
    this.ready.catch(() => {});
    this.stream.onmessage = message => {
      try { this.deliver(JSON.parse(message.data)); } catch { this.fail(new Error('Invalid Janus event envelope')); }
    };
    this.stream.onerror = () => this.fail(new Error('Janus event connection failed'));
  }
  record(phase, session, handle, operation) {
    if (this.trace.length < 4096) this.trace.push({ at: Date.now(), phase, session, handle, operation });
  }
  fail(error) {
    if (this.failed || this.closed) return;
    this.failed = error; this.failures.push(error.message); this.readyReject(error);
    clearTimeout(this.readyTimer); this.stream.close();
    for (const controller of this.controllers) controller.abort(error);
  }
  deliver(envelope) {
    if (envelope.error) { this.fail(new Error(envelope.error)); return; }
    const session = this.sessions.get(envelope.session_id);
    if (!session) {
      if (this.backlog.length >= 1024) { this.fail(new Error('Janus early-event overflow')); return; }
      this.backlog.push(envelope); return;
    }
    const event = envelope.event, pending = session.pending.get(event?.transaction);
    if (pending) { this.record('event', session.id, event.sender); pending.resolve(event); }
  }
  async api(path, body, signal) {
    const response = await this.fetcher('/janus' + path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), signal });
    const value = await response.json();
    if (!response.ok || value.janus === 'error') throw new Error('Janus request failed: ' + (value.error?.code ?? response.status));
    return value;
  }
  async operation(session, handle, body) {
    if (this.failed || this.closed) throw this.failed ?? new Error('Janus signal closed');
    const controller = new AbortController(), transaction = crypto.randomUUID();
    this.controllers.add(controller);
    const timer = setTimeout(() => controller.abort(new Error('Janus request/event deadline')), this.deadlineMs);
    let resolve, reject;
    const waiting = new Promise((ok, fail) => { resolve = ok; reject = fail; });
    // An event can arrive before the HTTP ACK. Observe rejection immediately.
    waiting.catch(() => {});
    let cancel;
    const cancelled = new Promise((_, fail) => { cancel = fail; }); cancelled.catch(() => {});
    const abort = () => { reject(controller.signal.reason); cancel(controller.signal.reason); };
    controller.signal.addEventListener('abort', abort, { once: true });
    if (session) session.pending.set(transaction, { resolve });
    const operation = body.body?.request ?? body.janus;
    this.record('request', session?.id, handle, operation);
    try {
      const posted = this.api(session ? `/${session.id}${handle ? '/' + handle : ''}` : '', { ...body, transaction }, controller.signal);
      // Deadline covers queued POST, response body, ACK and async event.
      const result = await Promise.race([posted, cancelled]);
      this.record('response', session?.id, handle, operation);
      const event = result.janus === 'ack' ? await Promise.race([waiting, cancelled]) : result;
      if (event.janus === 'error' || event.plugindata?.data?.error) throw new Error('Janus plugin request failed: ' + (event.plugindata?.data?.error_code ?? event.error?.code));
      return event;
    } catch (error) {
      this.record(controller.signal.aborted ? 'deadline-or-cancel' : 'error', session?.id, handle, operation); throw error;
    } finally {
      clearTimeout(timer); controller.signal.removeEventListener('abort', abort);
      session?.pending.delete(transaction); this.controllers.delete(controller);
    }
  }
  async session() {
    const response = await this.operation(null, null, { janus: 'create' });
    const entry = { id: response.data.id, pending: new Map() };
    entry.request = (handle, body) => this.operation(entry, handle, body);
    entry.attach = async () => (await entry.request(null, { janus: 'attach', plugin: 'janus.plugin.videoroom' })).data.id;
    this.sessions.set(entry.id, entry);
    const pending = this.backlog.filter(event => event.session_id === entry.id);
    this.backlog = this.backlog.filter(event => event.session_id !== entry.id); pending.forEach(event => this.deliver(event));
    return entry;
  }
  close() {
    this.closed = true; clearTimeout(this.readyTimer); this.stream.close();
    this.readyReject(new Error('Janus signal cancelled'));
    for (const controller of this.controllers) controller.abort(new Error('Janus signal cancelled'));
    this.backlog = [];
  }
}
