// The channel stage's choices for the call: the view picked by hand and
// the pinned picture. They last for the call, also when its page is left
// and opened again, and end with it.

import { create } from "zustand";

import { useVoice } from "./voiceStore.ts";

export type StageView = "raster" | "fokus";

type StageChoices = {
  /** `serverId/channelId` of the call the choices belong to. */
  call: string | null;
  /** Picked by hand; null follows the Lives (stageViewFor). */
  view: StageView | null;
  pinned: string | null;
};

const NONE: StageChoices = { call: null, view: null, pinned: null };

export const useStageChoices = create<StageChoices>(() => NONE);

/** This call's choices, or none yet. */
export function choicesFor(state: StageChoices, call: string): StageChoices {
  return state.call === call ? state : NONE;
}

export function chooseView(call: string, view: StageView): void {
  useStageChoices.setState((state) => ({
    ...choicesFor(state, call),
    call,
    view,
  }));
}

/** Pins a picture to the front, which is the Fokus view; null unpins. */
export function pinPicture(call: string, id: string | null): void {
  useStageChoices.setState((state) => {
    const current = choicesFor(state, call);
    return {
      call,
      pinned: id,
      view: id === null ? current.view : "fokus",
    };
  });
}

/** Fokus while someone else is live, unless a view was picked by hand. */
export function stageViewFor(
  picked: StageView | null,
  othersLive: boolean,
): StageView {
  return picked ?? (othersLive ? "fokus" : "raster");
}

// A call that ends takes its choices along.
useVoice.subscribe((state, previous) => {
  if (
    previous.status === "joined" &&
    (state.status !== "joined" ||
      state.serverId !== previous.serverId ||
      state.channelId !== previous.channelId)
  )
    useStageChoices.setState(NONE, true);
});
