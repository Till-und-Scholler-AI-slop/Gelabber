/** Clipboard write that reports success instead of throwing (denied
 * permission, insecure context). */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** Whether the system share sheet exists here (phones, some desktops). */
export function canShare(): boolean {
  return (
    typeof navigator !== "undefined" && typeof navigator.share === "function"
  );
}

/**
 * Opens the system share sheet. Resolves true once something was shared;
 * false when it was cancelled or there is no sheet.
 */
export async function share(data: ShareData): Promise<boolean> {
  if (!canShare()) return false;
  try {
    await navigator.share(data);
    return true;
  } catch {
    return false;
  }
}
