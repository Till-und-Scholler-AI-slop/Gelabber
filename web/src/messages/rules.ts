// Client-side mirror of `api/src/messages/validate.rs` and
// `api/src/attachments/validate.rs`.

import type { FieldCode } from "../api/client.ts";

export const CONTENT_MAX = 2000;
export const PAGE_SIZE = 50;
export const SIZE_MAX = 25 * 1024 * 1024;
export const FILENAME_MAX = 255;
/** Files per message and their total size (issue #255). */
export const ATTACHMENTS_MAX = 10;
export const MESSAGE_SIZE_MAX = 100 * 1024 * 1024;
/** Text files up to this size get a preview in the chat. */
export const TEXT_PREVIEW_MAX = 100 * 1024;

export const ALLOWED_TYPES = [
  "image/jpeg",
  "image/png",
  "image/gif",
  "image/webp",
  "application/pdf",
  "text/plain",
  "text/markdown",
  "text/csv",
  "application/json",
  "application/zip",
  "application/gzip",
  "application/x-tar",
  "application/x-7z-compressed",
  "audio/mpeg",
  "audio/wav",
  "audio/ogg",
  "audio/flac",
  "audio/mp4",
  "audio/aac",
  "audio/webm",
  "video/mp4",
  "video/webm",
  "video/quicktime",
] as const;

const IMAGE_TYPES = new Set([
  "image/jpeg",
  "image/png",
  "image/gif",
  "image/webp",
]);

/** Source code and other text: stored as text/plain, the name keeps the
 *  extension. HTML and SVG are deliberately absent. */
const TEXT_EXTENSIONS = [
  "txt",
  "log",
  "ini",
  "cfg",
  "conf",
  "toml",
  "yaml",
  "yml",
  "env",
  "rs",
  "py",
  "js",
  "mjs",
  "cjs",
  "ts",
  "tsx",
  "jsx",
  "c",
  "h",
  "cc",
  "cpp",
  "hpp",
  "cs",
  "java",
  "kt",
  "go",
  "rb",
  "php",
  "swift",
  "lua",
  "sh",
  "bash",
  "zsh",
  "fish",
  "ps1",
  "sql",
  "css",
  "scss",
  "diff",
  "patch",
  "nix",
  "zig",
  "hs",
  "ex",
  "exs",
  "dart",
  "r",
  "pl",
  "vue",
  "svelte",
  "gradle",
  "lock",
];

const EXT_TYPE: Record<string, string> = {
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  png: "image/png",
  gif: "image/gif",
  webp: "image/webp",
  pdf: "application/pdf",
  md: "text/markdown",
  markdown: "text/markdown",
  csv: "text/csv",
  json: "application/json",
  zip: "application/zip",
  gz: "application/gzip",
  tgz: "application/gzip",
  tar: "application/x-tar",
  "7z": "application/x-7z-compressed",
  mp3: "audio/mpeg",
  wav: "audio/wav",
  ogg: "audio/ogg",
  oga: "audio/ogg",
  opus: "audio/ogg",
  flac: "audio/flac",
  m4a: "audio/mp4",
  aac: "audio/aac",
  weba: "audio/webm",
  mp4: "video/mp4",
  m4v: "video/mp4",
  webm: "video/webm",
  mov: "video/quicktime",
  ...Object.fromEntries(TEXT_EXTENSIONS.map((ext) => [ext, "text/plain"])),
};

/** Other names browsers and systems use for an allowed type (same list as
 *  the API's). */
const ALIASES: Record<string, string> = {
  "image/jpg": "image/jpeg",
  "image/pjpeg": "image/jpeg",
  "audio/mp3": "audio/mpeg",
  "audio/x-mpeg": "audio/mpeg",
  "audio/x-wav": "audio/wav",
  "audio/wave": "audio/wav",
  "audio/vnd.wave": "audio/wav",
  "audio/x-flac": "audio/flac",
  "audio/x-m4a": "audio/mp4",
  "audio/m4a": "audio/mp4",
  "audio/x-aac": "audio/aac",
  "application/x-zip-compressed": "application/zip",
  "application/x-zip": "application/zip",
  "application/x-gzip": "application/gzip",
  "application/x-tgz": "application/gzip",
  "text/x-markdown": "text/markdown",
};

/** For the file picker: the types, and the extensions of files browsers
 *  give no type (source code). */
export const ACCEPT = [
  ...ALLOWED_TYPES,
  ...Object.keys(EXT_TYPE).map((ext) => `.${ext}`),
].join(",");

const chars = (value: string): number => Array.from(value).length;

/** Same normalisation as the API: CRLF → LF, trim, then the length check. */
export function normaliseContent(raw: string): string {
  return raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim();
}

export function validateContent(raw: string): FieldCode | null {
  const value = normaliseContent(raw);
  if (value.length === 0) return "required";
  if (chars(value) > CONTENT_MAX) return "too_long";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(value))
    return "invalid";
  return null;
}

const isAllowed = (type: string) =>
  (ALLOWED_TYPES as readonly string[]).includes(type);

/** The type to declare: the browser's when it is an allowed one, else the
 *  extension's. Browsers name source files `text/x-rust`,
 *  `text/javascript` or nothing at all; those become `text/plain`. */
export function inferContentType(file: { name: string; type: string }): string {
  const raw = file.type.trim().toLowerCase().split(";")[0]?.trim() ?? "";
  const typed = ALIASES[raw] ?? raw;
  if (isAllowed(typed)) return typed;
  const ext = file.name.split(".").pop()?.toLowerCase() ?? "";
  if (file.name.includes(".") && EXT_TYPE[ext]) return EXT_TYPE[ext];
  return typed;
}

export function isImageType(contentType: string): boolean {
  return IMAGE_TYPES.has(contentType);
}

export function isVideoType(contentType: string): boolean {
  return contentType.startsWith("video/");
}

export function isAudioType(contentType: string): boolean {
  return contentType.startsWith("audio/");
}

export function isTextType(contentType: string): boolean {
  return contentType.startsWith("text/") || contentType === "application/json";
}

/** A text file's bytes as text. The server accepts text in any encoding
 *  and serves it without a charset: UTF-16 by its byte order mark, then
 *  UTF-8, and anything else as Windows-1252 (Excel CSV on German systems,
 *  which also reads Latin-1 logs correctly). */
export function decodeText(bytes: ArrayBuffer | Uint8Array): string {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (view[0] === 0xff && view[1] === 0xfe)
    return new TextDecoder("utf-16le").decode(view);
  if (view[0] === 0xfe && view[1] === 0xff)
    return new TextDecoder("utf-16be").decode(view);
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(view);
  } catch {
    return new TextDecoder("windows-1252").decode(view);
  }
}

export function validateAttachment(file: {
  name: string;
  type: string;
  size: number;
}): { field: "filename" | "content_type" | "size"; code: FieldCode } | null {
  const name = file.name.replace(/^.*[/\\]/, "").trim();
  if (!name) return { field: "filename", code: "required" };
  if (chars(name) > FILENAME_MAX)
    return { field: "filename", code: "too_long" };
  const type = inferContentType(file);
  if (!type) return { field: "content_type", code: "required" };
  if (!isAllowed(type)) {
    return { field: "content_type", code: "invalid" };
  }
  if (file.size <= 0) return { field: "size", code: "required" };
  if (file.size > SIZE_MAX) return { field: "size", code: "too_long" };
  return null;
}

/** Limits of one message's files together; null when they fit. */
export function validateAttachmentSet(
  files: readonly { size: number }[],
): { field: "attachment_ids"; code: FieldCode } | null {
  if (files.length > ATTACHMENTS_MAX)
    return { field: "attachment_ids", code: "too_long" };
  const total = files.reduce((sum, file) => sum + file.size, 0);
  if (total > MESSAGE_SIZE_MAX)
    return { field: "attachment_ids", code: "too_large" };
  return null;
}

/** A file size as the chat shows it: "2,5 MB", "12 KB". */
export function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1).replace(".", ",")} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
