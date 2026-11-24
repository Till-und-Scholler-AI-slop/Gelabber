// Writes the licence file that ships next to deepfilternet.wasm: every crate
// linked into it (normal dependencies for wasm32, from `cargo metadata`)
// with its SPDX expression, and each distinct licence or notice file those
// crates carry, once. Run by build-deepfilternet.sh.
//
// Usage: node deepfilternet-licenses.mjs <metadata.json> <output.txt> <header.txt>
/* global console, process */
import fs from "node:fs";
import path from "node:path";

const [metadataPath, outputPath, headerPath] = process.argv.slice(2);
const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
const packages = new Map(metadata.packages.map((p) => [p.id, p]));
const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));

// Normal dependencies only: build scripts and proc macros run on the build
// machine and are not in the module.
const linked = new Set();
const queue = [metadata.resolve.root];
while (queue.length) {
  const id = queue.pop();
  if (linked.has(id)) continue;
  linked.add(id);
  for (const dep of nodes.get(id).deps)
    if (dep.dep_kinds.some((kind) => kind.kind === null)) queue.push(dep.pkg);
}
linked.delete(metadata.resolve.root);

const crates = [...linked]
  .map((id) => packages.get(id))
  .sort(
    (a, b) =>
      a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
  );
const texts = new Map();
for (const crate of crates) {
  const dir = path.dirname(crate.manifest_path);
  const files = fs
    .readdirSync(dir)
    .filter((name) => /^(licen[cs]e|copying|notice|copyright)/i.test(name))
    .sort();
  if (crate.license_file)
    files.push(path.relative(dir, path.resolve(dir, crate.license_file)));
  for (const file of new Set(files)) {
    const full = path.join(dir, file);
    if (!fs.statSync(full).isFile()) continue;
    const text = fs.readFileSync(full, "utf8").replace(/\r\n/g, "\n").trim();
    const entry = texts.get(text) ?? { users: [] };
    entry.users.push(`${crate.name} ${crate.version} (${file})`);
    texts.set(text, entry);
  }
}

const out = [fs.readFileSync(headerPath, "utf8").trim(), ""];
out.push("Crates linked into deepfilternet.wasm (name, version, licence):", "");
for (const crate of crates)
  out.push(
    `  ${crate.name} ${crate.version}: ${crate.license ?? `see ${crate.license_file}`}`,
  );
out.push(
  "",
  "Licence and notice files of these crates, each distinct text once:",
);
for (const [text, { users }] of texts) {
  out.push(
    "",
    "=".repeat(78),
    `Used by: ${users.join(", ")}`,
    "=".repeat(78),
    "",
    text,
  );
}
fs.writeFileSync(outputPath, `${out.join("\n")}\n`);
console.log(`${crates.length} crates, ${texts.size} distinct licence texts`);
