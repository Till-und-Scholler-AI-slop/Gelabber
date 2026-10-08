// IPC to the Gelabber desktop app's native media core (desktop/app).
//
// The desktop app loads this web client from the server and exposes only the
// `media_*` commands to it. In a browser none of this runs: `isDesktopApp()`
// is false and the Tauri API is never loaded.

export type NativeBridge = {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  /** A channel argument for a command; messages arrive in order. */
  channel<T>(onMessage: (message: T) => void): Promise<unknown>;
};

let override: NativeBridge | null | undefined;

function tauri(): NativeBridge {
  const core = import("@tauri-apps/api/core");
  return {
    async invoke<T>(command: string, args?: Record<string, unknown>) {
      return (await core).invoke<T>(command, args);
    },
    async channel<T>(onMessage: (message: T) => void) {
      const { Channel } = await core;
      return new Channel<T>(onMessage);
    },
  };
}

let tauriBridge: NativeBridge | null = null;

/** True inside the desktop app, whose native core replaces browser media. */
export function isDesktopApp(): boolean {
  if (override !== undefined) return override !== null;
  return (
    typeof window !== "undefined" &&
    typeof (window as unknown as { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__ === "object"
  );
}

export function nativeBridge(): NativeBridge {
  if (override) return override;
  if (!isDesktopApp()) throw new Error("native media is desktop-app only");
  tauriBridge ??= tauri();
  return tauriBridge;
}

export function invokeNative<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return nativeBridge().invoke<T>(command, args);
}

/** Tests: a fake bridge turns desktop mode on; `null` forces browser mode;
 * `undefined` restores detection. */
export function setNativeBridgeForTests(
  bridge: NativeBridge | null | undefined,
): void {
  override = bridge;
}
