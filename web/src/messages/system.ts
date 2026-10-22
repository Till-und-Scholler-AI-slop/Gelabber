// System messages (issue #314): rows the server writes on its own, sent as
// a kind plus parameters instead of a finished sentence, so the words are
// the client's (`api/src/messages/system.rs`). The author is the person the
// message is about; `content` is empty. A push uses the same wording
// (`system_text` in api/src/i18n.rs).

export type SystemMessage = {
  kind: string;
  params: Record<string, unknown>;
};

/** Redis cjson re-encoding may hand back anything; keep a usable shape. */
export function asSystem(value: unknown): SystemMessage | null {
  if (value === null || typeof value !== "object") return null;
  const system = value as Partial<SystemMessage>;
  if (typeof system.kind !== "string") return null;
  const params =
    system.params !== null &&
    typeof system.params === "object" &&
    !Array.isArray(system.params)
      ? system.params
      : {};
  return { kind: system.kind, params };
}

function param(system: SystemMessage, name: string): string {
  const value = system.params[name];
  return typeof value === "string" ? value : "";
}

/** The message as a sentence. A kind this page does not know yet (a newer
 *  server) is named as what it is. */
export function systemText(system: SystemMessage, author: string): string {
  switch (system.kind) {
    case "live_started":
      return `${author} ist live in ${param(system, "voice_name")}.`;
    default:
      return "Systemnachricht";
  }
}
