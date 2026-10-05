import { PCM } from './pcm-kernel.mjs';

export class PcmMarkers {
  static async create() {
    const context = new AudioContext({ sampleRate: PCM.sampleRate });
    try {
      await context.audioWorklet.addModule('/pcm.js');
      await context.resume();
      return new PcmMarkers(context);
    } catch (error) { await context.close(); throw error; }
  }
  constructor(context) { this.context = context; this.sources = new Map(); this.edges = []; this.nodes = []; this.failures = []; }
  source(name, number) {
    if (this.sources.has(name)) throw new Error('duplicate PCM marker source');
    const startFrame = Math.ceil((this.context.currentTime + .5) * PCM.sampleRate / 128) * 128;
    const entry = { name, number, startFrame, sent: [] }; this.sources.set(name, entry);
    const node = new AudioWorkletNode(this.context, 'gelabber-pcm-marker', { channelCount: 1, channelCountMode: 'explicit', outputChannelCount: [1], processorOptions: { mode: 'source', source: number, startFrame } });
    node.port.onmessage = ({ data }) => { if (data.type === 'sent') entry.sent.push(data); if (data.type === 'state') node.report?.(data); };
    node.onprocessorerror = () => this.failures.push('PCM source worklet failed: ' + name);
    this.nodes.push(node); return node;
  }
  receiver(track, peer, name, number) {
    const source = this.context.createMediaStreamSource(new MediaStream([track]));
    this.nodes.push(source); return this.receiverNode(source, peer, name, number);
  }
  receiverNode(input, peer, name, number) {
    const edge = { peer, source: name, number, received: [] }; this.edges.push(edge);
    const node = new AudioWorkletNode(this.context, 'gelabber-pcm-marker', { channelCount: 1, channelCountMode: 'explicit', outputChannelCount: [1], processorOptions: { mode: 'receiver', source: number, startFrame: this.sources.get(name)?.startFrame } });
    node.port.onmessage = ({ data }) => { if (data.type === 'received') edge.received.push(data); if (data.type === 'state') node.report?.(data); };
    node.onprocessorerror = () => this.failures.push('PCM receiver worklet failed: ' + peer + '/' + name);
    input.connect(node).connect(this.context.destination); // output is silent
    this.nodes.push(node); return edge;
  }
  begin() { this.firstFrame = Math.round(this.context.currentTime * PCM.sampleRate); this.firstWall = performance.now(); }
  end() { this.lastFrame = Math.round(this.context.currentTime * PCM.sampleRate); this.lastWall = performance.now(); }
  async evidence() {
    const states = await Promise.all(this.nodes.filter(node => node instanceof AudioWorkletNode).map(node => new Promise(resolve => {
      const deadline = setTimeout(() => { this.failures.push('PCM worklet state timeout'); resolve({}); }, 1000);
      node.report = data => { clearTimeout(deadline); resolve(data); }; node.port.postMessage('report');
    })));
    if (states.some(state => state.frame_gaps)) this.failures.push('PCM worklet sample-frame gap');
    const first = this.firstFrame ?? 0, last = this.lastFrame ?? Math.round(this.context.currentTime * PCM.sampleRate);
    const edges = this.edges.map(edge => {
      const source = this.sources.get(edge.source), sent = [];
      if (source) for (let frame = source.startFrame + Math.max(0, Math.ceil((first - source.startFrame) / PCM.periodFrames)) * PCM.periodFrames; frame <= last - PCM.maxDelayMs / 1000 * PCM.sampleRate; frame += PCM.periodFrames) {
        const event = source.sent.find(event => event.sentFrame === frame);
        sent.push(event ?? { sentFrame: frame, sequence: (frame - source.startFrame) / PCM.periodFrames, problem: 'planned source marker was not emitted' });
      }
      const matches = sent.map(event => {
        if (event.problem) return event;
        const candidates = edge.received.filter(received => received.sequence === event.sequence && received.receivedFrame >= event.sentFrame - PCM.errorBoundMs / 1000 * PCM.sampleRate && received.receivedFrame - event.sentFrame <= PCM.maxDelayMs / 1000 * PCM.sampleRate);
        if (candidates.length !== 1) return { sequence: event.sequence, sentFrame: event.sentFrame, problem: candidates.length ? 'ambiguous marker match' : 'marker not received' };
        const received = candidates[0]; return { sequence: event.sequence, sentFrame: event.sentFrame, ...received, latency_ms: (received.receivedFrame - event.sentFrame) / PCM.sampleRate * 1000 };
      });
      return { peer: edge.peer, source: edge.source, number: edge.number, expected_markers: sent.length, received: edge.received, matches };
    });
    return { scope: 'generated source PCM to decoded receiver PCM on one shared AudioContext sampleclock; excludes acoustic devices', parameters: PCM, firstFrame: first, lastFrame: last, sample_clock_seconds: (last - first) / PCM.sampleRate, wall_clock_seconds: (this.lastWall - this.firstWall) / 1000,
      sources: [...this.sources.values()], edges, worklet_states: states, failures: [...this.failures], clipped_frames: Math.max(0, ...states.map(state => state.clipped_frames ?? 0)) };
  }
  async close() { this.nodes.forEach(node => node.disconnect()); await this.context.close(); }
}
