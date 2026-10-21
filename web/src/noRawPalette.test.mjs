// Components take their colours from the Living Room tokens (--lr-*, or the
// lr-* Tailwind colours that map to them), so account themes reach every
// surface. Raw Tailwind palette classes ignore the theme. The files below
// still use them; each redesign package migrates its own and shrinks the
// list, and an entry that no longer needs to be here fails the test.
/* global URL */
import { readdirSync, readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = fileURLToPath(new URL(".", import.meta.url));
const RAW = /\b(?:bg-white|(?:neutral|red|emerald|amber)-\d{2,3})\b/g;

const ALLOWED = new Set([
  "components/AppShell.tsx",
  "components/CloseToTray.tsx",
  "components/EmojiPicker.tsx",
  "components/GifPicker.tsx",
  "components/InviteDialog.tsx",
  "components/MemberActions.tsx",
  "components/NotificationPermission.tsx",
  "components/NotificationPrivacy.tsx",
  "components/NotificationSettingsDialog.tsx",
  "components/ServerDialogs.tsx",
  "components/SessionRecoveryNotice.tsx",
  "pages/DmChannelPage.tsx",
  "pages/DmIndexPage.tsx",
  "pages/ForgotPasswordPage.tsx",
  "pages/InvitePage.tsx",
  "pages/LoginPage.tsx",
  "pages/NotFoundPage.tsx",
  "pages/ProfileCardForm.tsx",
  "pages/ProfilePage.tsx",
  "pages/RegisterPage.tsx",
  "pages/ResetPasswordPage.tsx",
  "pages/ServerSettingsPage.tsx",
  "pages/VerifyEmailPage.tsx",
  "pages/WorkspaceIndexPage.tsx",
]);

/** Component files by path under src, with the raw palette classes in each. */
function rawPaletteUse() {
  return readdirSync(src, { recursive: true, withFileTypes: true })
    .filter(
      (entry) =>
        entry.isFile() &&
        entry.name.endsWith(".tsx") &&
        !entry.name.includes(".test."),
    )
    .map((entry) => {
      const path = join(entry.parentPath, entry.name);
      return {
        file: relative(src, path).split(sep).join("/"),
        classes: [...new Set(readFileSync(path, "utf8").match(RAW) ?? [])],
      };
    });
}

describe("raw Tailwind palette classes", () => {
  const files = rawPaletteUse();

  it("finds the component files", () => {
    expect(files.map((f) => f.file)).toContain("components/Modal.tsx");
  });

  it("appear only in files that are not migrated yet", () => {
    const offenders = files
      .filter((f) => f.classes.length > 0 && !ALLOWED.has(f.file))
      .map((f) => `${f.file}: ${f.classes.join(", ")}`);
    expect(offenders).toEqual([]);
  });

  it("lists no file that is already migrated", () => {
    const clean = new Set(
      files.filter((f) => f.classes.length === 0).map((f) => f.file),
    );
    const known = new Set(files.map((f) => f.file));
    expect(
      [...ALLOWED].filter((file) => clean.has(file) || !known.has(file)),
    ).toEqual([]);
  });
});
