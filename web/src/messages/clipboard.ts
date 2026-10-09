// Files pasted into the composer. Screenshots arrive as clipboard items,
// copied files as `files`; WebKit may leave the name empty. Office apps put a
// picture of the selection next to its text: text wins then.

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

/** The first pasted file, or null when the paste carries text. */
export function pastedFile(
  data: ClipboardFiles | null | undefined,
  now = new Date(),
): File | null {
  if (!data || data.getData?.("text/plain").trim()) return null;
  let file: File | null = data.files?.[0] ?? null;
  for (let i = 0; !file && data.items && i < data.items.length; i++) {
    const item = data.items[i];
    if (item?.kind === "file") file = item.getAsFile();
  }
  if (!file || file.name.trim()) return file;
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
