/* global setTimeout, clearTimeout */
import { AsyncLocalStorage } from "node:async_hooks";
const budgets = new AsyncLocalStorage();
const unusablePages = new WeakSet();
const unusableProbes = new WeakSet();
export class NativeInterfaceFailure extends Error {
  constructor(metrics) {
    super("native-browser-interface-deadline");
    this.metrics = { ...metrics, testError: true };
  }
}
async function bounded(work, deadline, stage, invalidate) {
  const start = Date.now();
  if (start >= deadline) {
    invalidate();
    throw new NativeInterfaceFailure({
      stage: stage + "-already-expired",
      deadlineEpochMs: deadline,
      nativeDataAvailable: false,
    });
  }
  let timer;
  try {
    const result = await Promise.race([
      Promise.resolve().then(work),
      new Promise((_, reject) => {
        timer = setTimeout(
          () => {
            invalidate();
            reject(
              new NativeInterfaceFailure({
                stage,
                deadlineEpochMs: deadline,
                elapsedMs: Date.now() - start,
                nativeDataAvailable: false,
              }),
            );
          },
          Math.max(0, deadline - start),
        );
      }),
    ]);
    if (Date.now() > deadline) {
      invalidate();
      throw new NativeInterfaceFailure({
        stage: stage + "-late",
        deadlineEpochMs: deadline,
        nativeDataAvailable: false,
      });
    }
    return result;
  } finally {
    clearTimeout(timer);
  }
}
export async function deadlineProbe(probe, deadline) {
  if (unusableProbes.has(probe))
    throw new NativeInterfaceFailure({
      stage: "probe-quarantined",
      nativeDataAvailable: false,
    });
  const inherited = budgets.getStore();
  const end = Math.min(deadline, inherited?.deadline ?? deadline);
  const context = { deadline: end, cancel: new Set() };
  return budgets.run(context, () =>
    bounded(
      () => probe(Math.max(0, end - Date.now())),
      end,
      "outer-probe-await",
      () => {
        unusableProbes.add(probe);
        for (const cancel of context.cancel) cancel();
      },
    ),
  );
}
export async function nativeEvaluate(actor, fn, arg, deadlineMs = 5_000) {
  if (unusablePages.has(actor.page))
    throw new NativeInterfaceFailure({
      stage: "page-quarantined",
      nativeDataAvailable: false,
    });
  const deadline = Math.min(
    Date.now() + deadlineMs,
    budgets.getStore()?.deadline ?? Infinity,
  );
  const invalidate = () => {
    unusablePages.add(actor.page);
    actor.nativeEvaluationUnusable = true;
    // Promise.race cannot cancel native work. Request destruction exactly once;
    // never issue another evaluation on this page. finally handles the context.
    actor.nativeAbortClose ??= bounded(
      () => actor.page.close({ runBeforeUnload: false }),
      Date.now() + 5_000,
      "unusable-page-close",
      () => {},
    ).catch(() => false);
  };
  const context = budgets.getStore();
  context?.cancel.add(invalidate);
  try {
    const result = await bounded(
      () =>
        actor.page.evaluate(
          fn,
          fn.name === "sample" ? { deadlineEpochMs: deadline } : arg,
        ),
      deadline,
      "outer-page-evaluate",
      invalidate,
    );
    if (Date.now() > deadline) {
      invalidate();
      throw new NativeInterfaceFailure({
        stage: "outer-page-evaluate-late",
        nativeDataAvailable: false,
      });
    }
    return result;
  } catch (error) {
    if (error instanceof NativeInterfaceFailure) throw error;
    if (error?.message?.includes("E2E_NATIVE_STATS_DEADLINE")) {
      invalidate();
      throw new NativeInterfaceFailure({
        stage: "native-getStats",
        nativeDataAvailable: false,
        deadlineEpochMs: deadline,
      });
    }
    throw error;
  } finally {
    context?.cancel.delete(invalidate);
  }
}
