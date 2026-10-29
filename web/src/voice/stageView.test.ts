import { describe, expect, it } from "vitest";

import {
  choicesFor,
  chooseView,
  pinPicture,
  stageViewFor,
  useStageChoices,
} from "./stageView.ts";
import { IDLE_VOICE, useVoice } from "./voiceStore.ts";

const call = "a/stage";
const choices = () => choicesFor(useStageChoices.getState(), call);

describe("stage view", () => {
  it("goes to Fokus when someone else is live, unless a view was picked", () => {
    expect(stageViewFor(null, false)).toBe("raster");
    expect(stageViewFor(null, true)).toBe("fokus");
    expect(stageViewFor("raster", true)).toBe("raster");
    expect(stageViewFor("fokus", false)).toBe("fokus");
  });

  it("brings a pinned picture to the front, and keeps the view on unpinning", () => {
    pinPicture(call, "mia-v");
    expect(choices()).toMatchObject({ view: "fokus", pinned: "mia-v" });
    pinPicture(call, null);
    expect(choices()).toMatchObject({ view: "fokus", pinned: null });
    chooseView(call, "raster");
    expect(choices().view).toBe("raster");
    // Another call has none of these.
    expect(choicesFor(useStageChoices.getState(), "a/other")).toMatchObject({
      view: null,
      pinned: null,
    });
  });

  it("forgets the choices when the call ends", () => {
    useVoice.setState({
      ...IDLE_VOICE,
      status: "joined",
      serverId: "a",
      channelId: "stage",
    });
    chooseView(call, "fokus");
    pinPicture(call, "kim-s");
    useVoice.setState({ ...IDLE_VOICE });
    expect(choices()).toMatchObject({ view: null, pinned: null });
  });
});
