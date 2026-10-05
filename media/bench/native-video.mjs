// Private stdin/stdout controller; no listening port or credentials are exposed.
import { spawn } from 'node:child_process';
import readline from 'node:readline';

export function nativeExitEvidence(child) {
  return { exitcode: child.exitCode, signal: child.signalCode,
    clean: child.exitCode === 0 && child.signalCode === null };
}

export async function rethrowAfterCleanup(setupError, close) {
  try { await close(); }
  catch (cleanupError) {
    const error = new AggregateError([setupError, cleanupError], 'publisher setup and cleanup both failed', { cause: setupError });
    error.cleanup_errors = [String(cleanupError)];
    error.setup_error = String(setupError);
    error.cleanup_evidence = cleanupError.cleanup_evidence;
    throw error;
  }
  throw setupError;
}

export async function janusSessionAbsence(response) {
  // Janus returns an empty 404 body for an already destroyed session. Other
  // malformed responses remain parse errors rather than absence evidence.
  if (response.status === 404) return { absent: true, status: 404 };
  const value = await response.json();
  return { absent: value.janus === 'error' && value.error?.code === 458,
    status: response.status, janus_error_code: value.error?.code };
}

export class NativeVideo {
  constructor(binary, archive, bind = '127.0.0.1', args = ['--archive', archive, bind]) {
    this.child = spawn(binary, args, { stdio: ['pipe', 'pipe', 'inherit'] });
    this.serial = Promise.resolve();
    this.ready = new Promise((resolve, reject) => { this.initial = { resolve, reject }; });
    this.readyTimer = setTimeout(() => this.fail(new Error('native source startup deadline')), 30000);
    this.lines = readline.createInterface({ input: this.child.stdout });
    this.lines.on('line', line => {
      try {
        const value = JSON.parse(line), target = this.initial ?? this.pending;
        if (!target) throw new Error('unexpected native source response');
        if (this.initial && (!value.ready || !value.provenance)) throw new Error('invalid native source greeting');
        if (this.initial) { this.initial = undefined; clearTimeout(this.readyTimer); }
        else this.pending = undefined;
        value.error ? target.reject(new Error(value.error)) : target.resolve(value);
      } catch (error) { this.fail(error); }
    });
    this.child.on('error', error => this.fail(error));
    this.child.on('exit', (code, signal) => this.fail(new Error(`native source exited ${code}/${signal}`)));
    this.child.stdin.on('error', error => this.fail(error));
  }
  fail(error) {
    clearTimeout(this.readyTimer);
    this.initial?.reject(error); this.pending?.reject(error);
    this.initial = this.pending = undefined;
    this.failure = error;
  }
  call(request) {
    const execute = async () => {
      await this.ready;
      if (this.failure) throw this.failure;
      const result = new Promise((resolve, reject) => { this.pending = { resolve, reject }; });
      const timer = setTimeout(() => this.fail(new Error('native source RPC deadline')), 45000);
      this.child.stdin.write(JSON.stringify(request) + '\n');
      try { return await result; } finally { clearTimeout(timer); }
    };
    const result = this.serial.then(execute);
    this.serial = result.catch(() => {});
    return result;
  }
  async close() {
    clearTimeout(this.readyTimer);
    if (!this.child.pid || this.child.exitCode !== null || this.child.signalCode !== null) return;
    const stopped = new Promise(resolve => this.child.once('exit', resolve));
    // EOF/close causes the native component to cancel replay and close WebRTC.
    this.child.stdin.end(JSON.stringify({ op: 'close' }) + '\n');
    const timer = setTimeout(() => this.child.kill('SIGTERM'), 3000);
    const hard = setTimeout(() => this.child.kill('SIGKILL'), 5000);
    try { await stopped; } finally { clearTimeout(timer); clearTimeout(hard); this.lines.close(); }
  }
}

export function decodedVideo(samples, expectedBitrate) {
  const failures = [], edges = samples.map(sample => {
    const codecs = new Map(sample.stats.filter(s => s.type === 'codec').map(s => [s.id, s.mimeType?.toLowerCase()]));
    return sample.stats.filter(s => s.type === 'inbound-rtp' && s.kind === 'video' && s.packetsReceived > 0 && s.mid !== 'probator' && codecs.get(s.codecId) !== 'video/rtx');
  });
  if (edges.length < 2 || edges.some(edge => edge.length !== 1)) return { valid: false, failures: ['requires one advancing decoded video edge in every sample'] };
  const first = edges[0][0], last = edges.at(-1)[0], seconds = (last.timestamp - first.timestamp) / 1000;
  if (!(seconds > 0) || edges.some(edge => edge[0].id !== first.id)) return { valid: false, failures: ['video edge/clock changed during measurement'] };
  const bitrate = (last.bytesReceived - first.bytesReceived) * 8 / seconds;
  const fps = (last.framesDecoded - first.framesDecoded) / seconds;
  const received = last.packetsReceived - first.packetsReceived, lost = Math.max(0, last.packetsLost - first.packetsLost);
  if (!Number.isFinite(bitrate) || bitrate < .9 * expectedBitrate || bitrate > 1.1 * expectedBitrate) failures.push('actual received bitrate outside frozen source ±10%');
  if (!Number.isFinite(fps) || fps < 57 || fps > 63 || edges.some(edge => edge[0].frameWidth !== 1920 || edge[0].frameHeight !== 1080)) failures.push('requires actual decoded 1920x1080 at 60fps ±5%');
  if (!(received > 0) || !Number.isFinite(lost) || lost / (received + lost) > .01) failures.push('receiver packet loss exceeds 1% or counters are missing');
  for (let index = 1; index < edges.length; index++) {
    const previous = edges[index - 1][0], current = edges[index][0];
    if (!(current.framesDecoded > previous.framesDecoded) || !(current.bytesReceived > previous.bytesReceived)) failures.push('receiver decoder/RTP stalled');
  }
  return { valid: failures.length === 0, failures, measured_seconds: seconds, bitrate_bps: bitrate, decoded_fps: fps, lost_packets: lost, received_packets: received };
}
