// One media peer: socket, generation, SDP queue, ICE buffer.
// The seat and the watch each hold one. A chat-gateway reconnect must not
// replace a peer whose media transport is still alive. Close and error on
// that transport clear it, so the next reconnect can mint a new ticket.

import type { MediaClientFrame, MediaServerFrame, MediaSocket } from "./media.ts";
import type { PeerConnection } from "./session.ts";

export type IceCand = { candidate: string; sdpMid: string | null };

const OUTBOUND_CAP = 64;

export class MediaPeer {
  pc: PeerConnection | null = null;
  socket: MediaSocket | null = null;
  generation = 0;
  pendingIce: IceCand[] = [];
  makingOffer = false;
  sfuOffered = false;
  needOffer = false;
  /** SFU offer we have not answered yet. */
  sfuOfferOpen = false;
  /** First remote answer (or our answer to the SFU) has landed. */
  negotiated = false;
  /** SFU accepted `op:j` with `op:ok`. SDP and ICE stay queued until then. */
  accepted = false;
  private outbound: MediaClientFrame[] = [];
  private sdpChain: Promise<void> = Promise.resolve();
  private unbind: (() => void) | null = null;
  private unbindClose: (() => void) | null = null;
  private transportLive = false;

  /**
   * The media transport has not closed or failed.
   * A gateway blip must not replace this peer while that is true.
   */
  isOpen(): boolean {
    return this.socket !== null && this.transportLive;
  }

  enqueue(job: () => Promise<void>): Promise<void> {
    const run = this.sdpChain.then(job, job);
    this.sdpChain = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  signalingState(): string {
    return (
      this.pc?.signalingState ??
      (this.pc?.remoteDescription ? "have-remote-offer" : "stable")
    );
  }

  /**
   * Join goes out immediately. Every other frame waits until `accept`,
   * so a rejected join cannot be followed by ICE the SFU treats as unauthorized.
   */
  send(frame: MediaClientFrame): void {
    if (frame.op !== "j" && !this.accepted) {
      this.outbound.push(frame);
      if (this.outbound.length > OUTBOUND_CAP) this.outbound.shift();
      return;
    }
    this.socket?.send(frame);
  }

  /** `op:ok` — the SFU joined this peer. Flush signaling held back until then. */
  accept(): void {
    if (this.accepted) return;
    this.accepted = true;
    const queued = this.outbound;
    this.outbound = [];
    for (const frame of queued) this.socket?.send(frame);
  }

  bind(
    socket: MediaSocket,
    onFrame: (frame: MediaServerFrame) => void,
    onTransportDead?: () => void,
  ): void {
    this.unbind?.();
    this.unbindClose?.();
    this.socket = socket;
    this.transportLive = true;
    this.accepted = false;
    this.outbound = [];
    this.unbind = socket.onFrame(onFrame);
    this.unbindClose = socket.onClose(() => {
      if (this.socket !== socket) return;
      this.transportLive = false;
      this.socket = null;
      this.unbind?.();
      this.unbind = null;
      this.unbindClose = null;
      onTransportDead?.();
    });
  }

  /** Drop the socket and the peer connection. In-flight work sees a new generation. */
  close(): void {
    this.generation += 1;
    this.negotiated = false;
    this.pendingIce = [];
    this.outbound = [];
    this.accepted = false;
    this.sdpChain = Promise.resolve();
    this.makingOffer = false;
    this.sfuOffered = false;
    this.needOffer = false;
    this.sfuOfferOpen = false;
    this.transportLive = false;
    const unbindClose = this.unbindClose;
    this.unbindClose = null;
    unbindClose?.();
    this.unbind?.();
    this.unbind = null;
    const socket = this.socket;
    this.socket = null;
    socket?.close();
    this.pc?.close();
    this.pc = null;
  }
}
