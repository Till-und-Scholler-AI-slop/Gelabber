// One media peer: socket, generation, SDP queue, ICE buffer.
// The seat and the watch each hold one. A chat-gateway reconnect must not
// close a peer whose media socket is still open.

import type { MediaServerFrame, MediaSocket } from "./media.ts";
import type { PeerConnection } from "./session.ts";

export type IceCand = { candidate: string; sdpMid: string | null };

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
  private sdpChain: Promise<void> = Promise.resolve();
  private unbind: (() => void) | null = null;

  /** The media socket is up. A gateway blip must not replace this peer. */
  isOpen(): boolean {
    return this.socket !== null;
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

  bind(socket: MediaSocket, onFrame: (frame: MediaServerFrame) => void): void {
    this.unbind?.();
    this.socket = socket;
    this.unbind = socket.onFrame(onFrame);
  }

  /** Drop the socket and the peer connection. In-flight work sees a new generation. */
  close(): void {
    this.generation += 1;
    this.negotiated = false;
    this.pendingIce = [];
    this.sdpChain = Promise.resolve();
    this.makingOffer = false;
    this.sfuOffered = false;
    this.needOffer = false;
    this.sfuOfferOpen = false;
    this.unbind?.();
    this.unbind = null;
    this.socket?.close();
    this.socket = null;
    this.pc?.close();
    this.pc = null;
  }
}
