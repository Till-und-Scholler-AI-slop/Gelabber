// Private native peer0 controller. The separate binary keeps the prior
// video-only diagnostic source/CLI unchanged.
import { NativeVideo } from './native-video.mjs';

export class NativePeer extends NativeVideo {
  constructor(binary, video, mic, source, bind = '127.0.0.1') {
    super(binary, video, bind, ['--peer0', video, mic, source, bind]);
  }
  endpoint(peer) {
    return { call: request => this.call({ ...request, peer }) };
  }
}
