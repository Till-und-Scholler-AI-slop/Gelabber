/** Where a group of settings is kept, shown as a tag on the group. */
export type SettingsScope = "account" | "device";

export const SCOPE_LABEL: Record<SettingsScope, string> = {
  account: "Für deinen Account",
  device: "Nur dieses Gerät",
};
