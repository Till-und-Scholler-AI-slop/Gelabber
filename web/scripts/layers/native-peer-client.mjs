/* global document, MediaStream, AudioContext, RTCPeerConnection, RTCRtpSender, setInterval, clearInterval, fetch, setTimeout, clearTimeout */
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const post = (url, body) =>
  fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }).then((r) => {
    if (!r.ok) throw Error(`fixture ${r.status} ${url}`);
    return r.json();
  });
export class NativePeer {
  constructor() {
    this.pc = new RTCPeerConnection({ iceServers: [] });
    this.resources = [];
    this.videos = [];
    this.tracks = new Set();
    this.offers = [];
    this.sources = new Map();
    this.signalQueue = Promise.resolve();
  }
  signal(task) {
    const result = this.signalQueue.then(task);
    this.signalQueue = result.catch(() => {});
    return result;
  }
  capture(index = 0) {
    const canvas = document.createElement("canvas");
    canvas.width = 640;
    canvas.height = 360;
    const ctx = canvas.getContext("2d");
    let frame = index * 30;
    const draw = () => {
      ctx.fillStyle = `rgb(${frame++ % 255},80,180)`;
      ctx.fillRect(0, 0, 640, 360);
      ctx.fillStyle = "white";
      ctx.fillRect((frame * 9) % 600, 20, 30, 30);
    };
    draw();
    const timer = setInterval(draw, 33),
      stream = canvas.captureStream(30),
      track = stream.getVideoTracks()[0];
    const dispose = () => {
      clearInterval(timer);
      track.stop();
    };
    this.resources.push(dispose);
    return { stream, track, dispose };
  }
  async start({ url, kinds, version = 3, legacy = false }) {
    const sources = [];
    if (legacy === "vp9")
      this.pc.addTransceiver("video", { direction: "recvonly" });
    for (const [index, kind] of kinds.entries()) {
      const capture = this.capture(index),
        { stream, track } = capture;
      let sender;
      if (legacy === true) {
        sender = this.pc.addTrack(track, stream);
      } else if (legacy === "vp9") {
        const transceiver = this.pc.addTransceiver(track, {
          direction: "sendrecv",
          streams: [stream],
        });
        const codecs = RTCRtpSender.getCapabilities("video").codecs.filter(
          (c) =>
            c.mimeType.toLowerCase() === "video/vp9" &&
            (!/profile-id=/.test(c.sdpFmtpLine ?? "") ||
              /profile-id=0(?:;|$)/.test(c.sdpFmtpLine)),
        );
        if (!codecs.length) throw Error("native VP9 profile0 missing");
        transceiver.setCodecPreferences(codecs);
        sender = transceiver.sender;
      } else {
        sender = globalThis.__layerModule.addLayeredVideo(
          this.pc,
          track,
          stream,
        );
        if (!sender) throw Error("actual layered helper missing");
      }
      sources.push({ kind, sender });
      this.sources.set(kind, { ...capture, sender });
    }
    const context = new AudioContext(),
      tone = context.createOscillator(),
      destination = context.createMediaStreamDestination();
    tone.connect(destination);
    tone.start();
    await context.resume();
    this.pc.addTrack(
      destination.stream.getAudioTracks()[0],
      destination.stream,
    );
    this.resources.push(async () => {
      tone.stop();
      destination.stream.getTracks().forEach((t) => t.stop());
      await context.close();
    });
    this.pc.ontrack = (e) => {
      this.tracks.add(e.track.id);
      if (
        e.track.kind === "video" &&
        !this.videos.some((v) => v.srcObject.getVideoTracks().includes(e.track))
      ) {
        const video = document.createElement("video");
        video.muted = true;
        video.autoplay = true;
        video.style = "width:640px;height:360px";
        video.dataset.source = e.streams[0]?.id ?? "";
        video.srcObject = new MediaStream([e.track]);
        document.body.append(video);
        void video.play();
        this.videos.push(video);
      }
    };
    await this.pc.setLocalDescription(await this.pc.createOffer());
    if (this.pc.iceGatheringState !== "complete")
      await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(Error("ICE deadline")), 10000);
        this.pc.onicegatheringstatechange = () => {
          if (this.pc.iceGatheringState === "complete") {
            clearTimeout(timeout);
            resolve();
          }
        };
      });
    const answer = await post(url, {
      sdp: this.pc.localDescription.sdp,
      sources: sources.map(({ kind, sender }) => {
        const mid = this.pc
          .getTransceivers()
          .find((t) => t.sender === sender)?.mid;
        const track = globalThis
          .__publishedTrackIds(this.pc.localDescription.sdp)
          .get(mid);
        if (!track) throw Error("publisher MID has no actual MSID");
        return { kind, track };
      }),
      version,
    });
    Object.assign(this, answer);
    await this.pc.setRemoteDescription({ type: "answer", sdp: answer.sdp });
    this.done = false;
    this.pump = (async () => {
      while (!this.done) {
        for (const message of await fetch(`/poll/${this.probe}`).then((r) =>
          r.json(),
        )) {
          if (message.op === "o") {
            await this.signal(async () => {
              await this.pc.setRemoteDescription({
                type: "offer",
                sdp: message.sdp,
              });
              await this.pc.setLocalDescription(await this.pc.createAnswer());
              this.offers.push({
                offer: message.sdp,
                answer: this.pc.localDescription.sdp,
              });
              await post(`/answer/${this.probe}`, {
                sdp: this.pc.localDescription.sdp,
              });
            });
          } else if (message.op === "err") throw Error(`SFU ${message.e}`);
        }
        await wait(30);
      }
    })().catch((error) => {
      this.error = String(error);
    });
    return { probe: this.probe, user: this.user };
  }
  publication(kind) {
    const capture = this.sources.get(kind);
    if (!capture) throw Error(`unknown owned source ${kind}`);
    const mid = this.pc
      .getTransceivers()
      .find((t) => t.sender === capture.sender)?.mid;
    const track = globalThis
      .__publishedTrackIds(this.pc.localDescription.sdp)
      .get(mid);
    if (!track) throw Error(`missing actual source MSID ${kind}`);
    return {
      kind,
      track,
      mid,
      capture: capture.track.id,
      ready: capture.track.readyState,
    };
  }
  async stable() {
    for (let n = 0; n < 100; n++) {
      if (this.error) throw Error(this.error);
      const server = await fetch(`/debug/${this.probe}`).then((r) => r.json());
      if (this.pc.signalingState === "stable" && !server.haveLocalOffer) return;
      await wait(30);
    }
    throw Error(
      `source restart signaling did not settle ${JSON.stringify(await this.debug())}`,
    );
  }
  async reofferLocked(kinds = []) {
    await this.pc.setLocalDescription(await this.pc.createOffer());
    const answer = await post(`/renegotiate/${this.probe}`, {
      sdp: this.pc.localDescription.sdp,
      sources: kinds.map((kind) => this.publication(kind)),
    });
    await this.pc.setRemoteDescription({ type: "answer", sdp: answer.sdp });
  }
  async stopSource(kind, strategy = "replace") {
    await this.stable();
    return this.signal(async () => {
      const identity = this.publication(kind),
        capture = this.sources.get(kind);
      await post(`/retract/${this.probe}`, identity);
      if (strategy === "replace") await capture.sender.replaceTrack(null);
      else this.pc.removeTrack(capture.sender);
      capture.dispose();
      await this.reofferLocked();
      return { ...identity, stopped: capture.track.readyState };
    });
  }
  async restartSource(kind) {
    await this.stable();
    return this.signal(async () => {
      const previous = this.sources.get(kind),
        fresh = this.capture(7);
      const transceiver = this.pc
        .getTransceivers()
        .find((t) => t.sender === previous.sender);
      // Same sender/direction reuse as the product's publishVideo path.
      const sender = previous.sender;
      await sender.replaceTrack(fresh.track);
      if (transceiver.direction === "recvonly")
        transceiver.direction = "sendrecv";
      else if (transceiver.direction === "inactive")
        transceiver.direction = "sendonly";
      this.sources.set(kind, { ...fresh, sender });
      await this.reofferLocked([kind]);
      return this.publication(kind);
    });
  }
  async freshSource(source, before) {
    for (let n = 0; n < 100; n++) {
      const sample = await this.stats(),
        server = await fetch(`/debug/${this.probe}`).then((r) => r.json());
      const binding = server.bindings.find((b) => b.source === source);
      const video = sample.find(
        (s) =>
          s.type === "inbound-rtp" &&
          s.kind === "video" &&
          s.ssrc === binding?.ssrc,
      );
      const baseline = before.find(
        (s) => s.type === "inbound-rtp" && s.ssrc === video?.ssrc,
      );
      const audio = sample
        .filter((s) => s.type === "inbound-rtp" && s.kind === "audio")
        .reduce((sum, s) => sum + s.packets, 0);
      const oldAudio = before
        .filter((s) => s.type === "inbound-rtp" && s.kind === "audio")
        .reduce((sum, s) => sum + s.packets, 0);
      if (
        video?.height > 0 &&
        video.frames >= (baseline?.frames ?? 0) + 5 &&
        video.packets >= (baseline?.packets ?? 0) + 5 &&
        audio >= oldAudio + 5
      )
        return { source, video, audio, binding };
      await wait(80);
    }
    throw Error(
      `new source has no fresh matching decoder ${JSON.stringify(await this.stats())} ${JSON.stringify(await this.debug())}`,
    );
  }
  async stats() {
    if (this.error) throw Error(this.error);
    const stats = await this.pc.getStats();
    return Array.from(stats.values())
      .filter((s) => s.type === "inbound-rtp" || s.type === "outbound-rtp")
      .map((s) => ({
        type: s.type,
        kind: s.kind,
        rid: s.rid,
        ssrc: s.ssrc,
        codec: stats.get(s.codecId)?.mimeType,
        fmtp: stats.get(s.codecId)?.sdpFmtpLine,
        height: s.frameHeight,
        frames: s.framesDecoded ?? s.framesEncoded,
        packets: s.packetsReceived ?? s.packetsSent,
        lost: s.packetsLost,
        track: s.trackIdentifier,
      }));
  }
  async debug() {
    return {
      localIdentifiers: this.pc.localDescription.sdp
        .split(/\r?\n/)
        .filter((s) =>
          /^(m=|a=(?:mid:|msid:|ssrc:|ssrc-group:|rid:|simulcast:|extmap:))/.test(
            s,
          ),
        ),
      offers: this.offers.map(({ offer, answer }) => ({
        offer: offer
          .split(/\r?\n/)
          .filter((s) =>
            /^(m=|a=(?:mid:|msid:|rtpmap:|fmtp:|rid:|simulcast:|sendrecv|sendonly|recvonly|inactive))/.test(
              s,
            ),
          ),
        answer: answer
          .split(/\r?\n/)
          .filter((s) =>
            /^(m=|a=(?:mid:|msid:|rtpmap:|fmtp:|rid:|simulcast:|sendrecv|sendonly|recvonly|inactive))/.test(
              s,
            ),
          ),
      })),
      server: await fetch(`/debug/${this.probe}`).then((r) => r.json()),
      senders: this.pc.getSenders().map((sender) => ({
        track: sender.track?.id,
        parameters: sender.getParameters(),
      })),
      native: this.pc
        .getTransceivers()
        .map((t) => ({ mid: t.mid, direction: t.currentDirection })),
      videos: this.videos.length,
      videoSources: this.videos.map((v) => v.dataset.source),
      tracks: this.tracks.size,
    };
  }
  async decoded(
    height,
    previous = { frames: 0, audio: 0 },
    withAudio = true,
    minimumHeight = false,
    source = undefined,
  ) {
    let sample;
    for (let n = 0; n < 100; n++) {
      sample = await this.stats();
      const sourceTracks =
        source == null
          ? undefined
          : new Set(
              this.videos
                .filter((v) => v.dataset.source === source)
                .flatMap((v) => v.srcObject.getVideoTracks().map((t) => t.id)),
            );
      const totalFrames = sample
        .filter(
          (s) =>
            s.type === "inbound-rtp" &&
            s.kind === "video" &&
            (sourceTracks == null || sourceTracks.has(s.track)),
        )
        .reduce((sum, s) => sum + (s.frames ?? 0), 0);
      const video = sample.find(
        (s) =>
          s.type === "inbound-rtp" &&
          s.kind === "video" &&
          (sourceTracks == null || sourceTracks.has(s.track)) &&
          (height === 0
            ? s.height > 0
            : minimumHeight
              ? s.height >= height
              : s.height === height),
      );
      if (totalFrames < previous.frames + 5) {
        await wait(80);
        continue;
      }
      const audio = sample
        .filter((s) => s.type === "inbound-rtp" && s.kind === "audio")
        .reduce((sum, s) => sum + (s.packets ?? 0), 0);
      if (video && (!withAudio || audio >= previous.audio + 5))
        return { frames: totalFrames, audio, track: video.track, sample };
      await wait(80);
    }
    throw Error(
      `native decode ${height}px failed ${JSON.stringify(sample)} debug=${JSON.stringify(await this.debug())}`,
    );
  }
  async controller(publisher, kind, expected) {
    const controller = new globalThis.__layerModule.ViewerLayerController();
    for (let n = 0; n < 100; n++) {
      const report = Array.from((await this.pc.getStats()).values());
      const sources = this.videos
        .filter((v) => v.dataset.source === `${publisher.user}:${kind}`)
        .flatMap((v) =>
          v.srcObject
            .getVideoTracks()
            .map((t) => ({ trackId: t.id, userId: publisher.user, kind })),
        );
      const hints = [];
      controller.update(report, sources, (hint) => hints.push(hint));
      for (const hint of hints)
        await post(`/layer/${this.probe}`, {
          publisher: publisher.probe,
          kind,
          height: hint.h,
          congested: hint.congested,
        });
      if (hints.some((hint) => hint.h === expected)) {
        await wait(500);
        return hints;
      }
      await wait(80);
    }
    throw Error(
      "actual viewer controller did not associate a native inbound track",
    );
  }
  async stop() {
    this.done = true;
    if (this.pump) await this.pump;
    this.pc.close();
    for (const dispose of this.resources) await dispose();
    this.videos.forEach((v) => v.remove());
  }
}
