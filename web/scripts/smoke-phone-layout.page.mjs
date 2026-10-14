// Page side of smoke-phone-layout.mjs, which builds it together with the
// app, so that both share the app's modules.
/* global window */
import { useVoice } from "../src/voice/session.ts";

/** Puts the phone into a call or a stream without a media server: the
 * app's call state, idle except for `state`. */
window.smokeCall = (state) =>
  useVoice.setState({ ...useVoice.getInitialState(), ...state });
