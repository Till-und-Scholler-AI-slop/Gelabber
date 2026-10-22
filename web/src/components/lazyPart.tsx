// Parts of the app that load after the first screen. Each is its own chunk:
// it starts loading when it is first shown, or when the workspace is idle.
//
// No automatic page reload on a failed load. After a deploy this tab may ask
// for a chunk the server no longer has, and a reload would end a running
// call. The part says so and offers a reload only while no call runs. There
// is no retry: browsers keep a failed module fetch for the page's lifetime.
// The idle preload is what keeps this rare.

import { useEffect, useState, type ComponentType, type ReactNode } from "react";

import { LazyPartFailed } from "./LazyPartFailed.tsx";

export type LazyPart<P extends object> = {
  /** Shows the part, or `fallback` until its chunk is there. */
  Component: ComponentType<P & { fallback?: ReactNode }>;
  /** Starts loading the chunk; settles once it is there or has failed. */
  preload: () => Promise<void>;
  /** Tests: whether the chunk is loaded. */
  loaded: () => boolean;
};

const parts: LazyPart<never>[] = [];

/** `failure` is the sentence shown when the chunk cannot be loaded. */
export function lazyPart<P extends object>(
  load: () => Promise<ComponentType<P>>,
  failure: string,
): LazyPart<P> {
  let ready: ComponentType<P> | null = null;
  let pending: Promise<ComponentType<P>> | null = null;
  const start = () =>
    (pending ??= load().then((component) => (ready = component)));

  function Component({
    fallback = null,
    ...props
  }: P & { fallback?: ReactNode }) {
    const [state, setState] = useState<{
      ready: ComponentType<P> | null;
      failed: boolean;
    }>(() => ({ ready, failed: false }));
    useEffect(() => {
      if (state.ready || state.failed) return;
      let alive = true;
      start().then(
        (component) => {
          if (alive) setState({ ready: component, failed: false });
        },
        () => {
          if (alive) setState({ ready: null, failed: true });
        },
      );
      return () => {
        alive = false;
      };
    }, [state.ready, state.failed]);
    if (state.ready) {
      const Ready = state.ready;
      return <Ready {...(props as P)} />;
    }
    if (state.failed) return <LazyPartFailed failure={failure} />;
    return fallback;
  }
  Component.displayName = "LazyPart";

  const part: LazyPart<P> = {
    Component,
    preload: () =>
      start().then(
        () => undefined,
        () => undefined,
      ),
    loaded: () => ready !== null,
  };
  parts.push(part as unknown as LazyPart<never>);
  return part;
}

/** Loads every part that is not loaded yet, once the browser is idle, so a
 * later deploy cannot take a chunk away from this tab. */
export function preloadPartsWhenIdle(): () => void {
  const run = () => {
    for (const part of parts) if (!part.loaded()) void part.preload();
  };
  if (typeof window.requestIdleCallback === "function") {
    const id = window.requestIdleCallback(run, { timeout: 5_000 });
    return () => window.cancelIdleCallback(id);
  }
  const id = window.setTimeout(run, 2_000);
  return () => window.clearTimeout(id);
}
