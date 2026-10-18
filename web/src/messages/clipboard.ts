// Files pasted or dropped into the composer. Screenshots arrive as
// clipboard items, copied files as `files`; WebKit may leave the name empty.
// Office apps put a picture of the selection next to its text: on paste, text
// wins then.

type ClipboardFiles = {
  getData?(format: string): string;
  files?: ArrayLike<File> | null;
  items?: ArrayLike<{ kind: string; getAsFile(): File | null }> | null;
};

const EXT: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
};

/** The pasted files, none when the paste carries text. */
export function pastedFiles(
  data: ClipboardFiles | null | undefined,
  now = new Date(),
): File[] {
  if (!data || data.getData?.("text/plain").trim()) return [];
  let files = Array.from(data.files ?? []);
  if (files.length === 0)
    files = Array.from(data.items ?? [])
      .filter((item) => item.kind === "file")
      .map((item) => item.getAsFile())
      .filter((file): file is File => file !== null);
  return files.map((file) => named(file, now));
}

/** The first pasted file, or null when the paste carries text. */
export function pastedFile(
  data: ClipboardFiles | null | undefined,
  now = new Date(),
): File | null {
  return pastedFiles(data, now)[0] ?? null;
}

/** The dropped files; a drag of text or links carries none. */
export function droppedFiles(
  data: ClipboardFiles | null | undefined,
  now = new Date(),
): File[] {
  return Array.from(data?.files ?? []).map((file) => named(file, now));
}

/** The first dropped file. */
export function droppedFile(
  data: ClipboardFiles | null | undefined,
  now = new Date(),
): File | null {
  return droppedFiles(data, now)[0] ?? null;
}

/** Whether a drag carries files, readable before the drop. */
export function carriesFiles(
  data: { types?: ArrayLike<string> } | null | undefined,
): boolean {
  return Array.from(data?.types ?? []).includes("Files");
}

function named(file: File, now: Date): File {
  if (file.name.trim()) return file;
  const stamp = now
    .toISOString()
    .slice(0, 19)
    .replace(/[-:]/g, "")
    .replace("T", "-");
  const ext = EXT[file.type] ?? "bin";
  return new File([file], `Bild-${stamp}.${ext}`, {
    type: file.type,
    lastModified: file.lastModified,
  });
}
