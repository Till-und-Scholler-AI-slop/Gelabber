export type MediaPriority = "very-low" | "low" | "medium" | "high";
export type PriorityParameters = {
  encodings: Array<{
    priority?: MediaPriority;
    networkPriority?: MediaPriority;
  }>;
};
type PrioritySender<T extends PriorityParameters> = {
  getParameters?: () => T;
  setParameters?: (parameters: T) => Promise<void>;
};
export type SenderPriority = {
  priority: MediaPriority | null;
  networkPriority: MediaPriority | null;
};

/** Priority changes are isolated from bitrate/codec updates and never touch capture. */
export async function prioritizeSender<T extends PriorityParameters>(
  sender: PrioritySender<T>,
  kind: "audio" | "video",
  current: () => boolean = () => true,
): Promise<void> {
  const desired = kind === "audio" ? "high" : "low";
  for (const field of ["priority", "networkPriority"] as const) {
    if (!current()) return;
    try {
      const parameters = sender.getParameters?.();
      if (!parameters?.encodings.length || !sender.setParameters) return;
      let changed = false;
      for (const encoding of parameters.encodings) {
        if (field === "networkPriority" && !Object.hasOwn(encoding, field))
          continue;
        if (encoding[field] === desired) continue;
        encoding[field] = desired;
        changed = true;
      }
      // Keep the fresh transactionId and every existing peer/codec/cap parameter.
      if (changed) await sender.setParameters(parameters);
    } catch {
      // Unsupported/rejected hints leave the sender and its last working settings alive.
    }
  }
}

export function readSenderPriority<T extends PriorityParameters>(
  sender: PrioritySender<T>,
): SenderPriority {
  try {
    const encoding = sender.getParameters?.().encodings[0];
    return {
      priority: encoding?.priority ?? null,
      networkPriority: encoding?.networkPriority ?? null,
    };
  } catch {
    return { priority: null, networkPriority: null };
  }
}
