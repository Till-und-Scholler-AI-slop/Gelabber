#!/usr/bin/env python3
"""Writes desktop/app/package/THIRD-PARTY-NOTICES.txt: the third-party
software built into the desktop packages, with the licence texts it ships.

  desktop/packaging/third-party-notices.py
      Rust parts afresh from desktop/Cargo.lock (cargo tree, cargo metadata,
      the crate sources in the cargo registry and the licence texts of the
      pinned Rust toolchain). The two parts on the native media core are
      kept from the existing file as long as their pins have not changed
      and their text still has the digest written next to them.
  desktop/packaging/third-party-notices.py \\
      --webrtc-src <work-dir>/src --core-build <cmake build dir of the core>
      Everything afresh. <work-dir> is the one of build-libwebrtc-linux.sh
      (a finished Linux build, out/gelabber); the CMake build directory is
      <target>/<profile>/build/gelabber-media-core-*/out/build and holds
      the unstripped libgelabber_media.so of that build. --core-build alone
      renews only the part that does not need the libwebrtc checkout.
  desktop/packaging/third-party-notices.py --check
      Exit status 1 when the committed file is not what the first form
      would write, or when a kept part was changed by hand.

Runs on Linux x86-64 only: cargo works out what build scripts and macros
depend on for the machine it runs on, so the list of build-time crates is
defined as the one of that host. Needs python3, git, cargo and the rustup
toolchain of rust-toolchain.toml, for everything afresh also nm; no network
beyond what cargo needs to read the crates of the lock file. Nothing is
written outside the output file.

Tests: python3 -m unittest discover -s desktop/packaging/tests
"""

import argparse
import hashlib
import html
import importlib.util
import json
import os
import re
import struct
import subprocess
import sys
import tarfile
import tempfile
import textwrap
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
NATIVE = REPO / "desktop/native"
OUTPUT = REPO / "desktop/app/package/THIRD-PARTY-NOTICES.txt"
GELABBER_URL = "https://github.com/Till-und-Scholler-AI-slop/Gelabber"
TARGETS = {"x86_64-unknown-linux-gnu": "Linux", "x86_64-pc-windows-msvc": "Windows"}
HOST = "x86_64-unknown-linux-gnu"
CORE_LIBRARY = "libgelabber_media.so"
WIDTH = 78

# Raise when the wording or layout of a kept part changes, so that an old
# copy of it is not taken over.
LIBWEBRTC_PART = 2
CORE_PART = 1

# The build configuration the fixed sentences of parts 1 and 2 describe.
# The script stops when the repository says something else.
EXPECTED_FACTS = {
    "Linux libwebrtc is built with rtc_use_h264=true and FFmpeg's Chrome branding": True,
    "Windows libwebrtc is built with rtc_use_h264=true": True,
    "Windows libwebrtc is built with use_custom_libcxx=false": True,
    "the core defines WEBRTC_USE_H264 on Linux only": True,
    "the core links the static MSVC runtime on Windows": True,
    "the Windows installer embeds the WebView2 bootstrapper": True,
}

LICENCE_NAME = re.compile(r"(licen[sc]e|copying|copyright|notice|unlicense|patents)", re.I)
# Source files whose name begins like a licence file (protobuf's notices.h).
CODE = (".h", ".c", ".cc", ".cpp", ".py", ".gn", ".gni", ".asm", ".S", ".inc", ".js", ".java", ".json")

# Files that stand next to a library's licence file in libwebrtc's tree and
# that neither libwebrtc's generator nor the library's README.chromium names,
# looked at by hand: True prints the file, a sentence says why it is not
# printed. A licence file found there without an entry stops the script,
# unless its text is already part of a printed one.
FFMPEG_OTHER_CONFIGURATIONS = (
    "FFmpeg's licence texts for builds configured with --enable-gpl or --enable-version3. "
    "The config.h of this build says CONFIG_GPL 0 and CONFIG_VERSION3 0."
)
BESIDE_THE_LICENCE = {
    "PATENTS": True,
    "license_template.txt": (
        "The comment that WebRTC's own source files begin with; it refers to LICENSE and "
        "PATENTS."
    ),
    "third_party/ffmpeg/COPYING.LGPLv2.1": True,
    "third_party/ffmpeg/COPYING.GPLv2": FFMPEG_OTHER_CONFIGURATIONS,
    "third_party/ffmpeg/COPYING.GPLv3": FFMPEG_OTHER_CONFIGURATIONS,
    "third_party/ffmpeg/COPYING.LGPLv3": FFMPEG_OTHER_CONFIGURATIONS,
    # LICENSE.md names it as the place of the IJG licence.
    "third_party/libjpeg_turbo/README.ijg": True,
    "third_party/libyuv/PATENTS": True,
    "third_party/opus/src/LICENSE_PLEASE_READ.txt": True,
}

# The compiler's own header directory in libwebrtc's tree: files of it that
# the build read are not a library missing from the list.
COMPILER_DIRECTORY = "third_party/llvm-build/"

# Wording by which a licence is recognised in a printed text, to compare a
# library's licence file with what its README.chromium calls it.
WORDING = {
    "Apache-2.0": r"Apache License,? Version 2\.0",
    "the LLVM exceptions": r"LLVM Exceptions to the Apache 2\.0 License",
    "NCSA": r"to deal with the Software without restriction",
    "MIT": r"to deal in the Software without restriction",
    "BSD": r"Redistribution and use in source and binary forms",
    "ISC": (
        r"Permission to use, copy, modify, and(/or)? distribute this software for any purpose "
        r"with or without fee is hereby granted"
    ),
    "LGPL-2.1": r"GNU LESSER GENERAL PUBLIC LICENSE Version 2\.1",
    "IJG": r"The authors make NO WARRANTY or representation",
    "Zlib": r"The origin of this software must not be misrepresented",
    "the SQLite blessing": r"In place of a legal notice, here is a blessing",
    "OpenSSL": r"OpenSSL",
    "SSLeay": r"SSLeay",
    "a statement on patents": r"(?i)\bpatents?\b",
}
# Looked for in every text; the others only where README.chromium names them.
ALWAYS_LOOKED_FOR = (
    "Apache-2.0", "the LLVM exceptions", "NCSA", "MIT", "BSD", "ISC", "LGPL-2.1", "IJG", "Zlib",
    "the SQLite blessing",
)
# What the licence names in the README.chromium files stand for in WORDING.
# Names that libwebrtc's tree gives to one-off texts stand for nothing that
# could be looked for. A name without an entry stops the script.
NAMED_LICENCES = {
    "Apache-2.0": ["Apache-2.0"],
    "Apache-with-LLVM-Exception": ["Apache-2.0", "the LLVM exceptions"],
    "BSD": ["BSD"], "BSD-2-Clause": ["BSD"], "BSD-3-Clause": ["BSD"], "BSD-3": ["BSD"],
    "3-clause BSD": ["BSD"],
    "MIT": ["MIT"], "NCSA": ["NCSA"], "ISC": ["ISC"], "LGPL 2.1": ["LGPL-2.1"],
    "IJG": ["IJG"], "Zlib": ["Zlib"], "blessing": ["the SQLite blessing"],
    "OpenSSL": ["OpenSSL"], "SSLeay": ["SSLeay"],
    "Patent": ["a statement on patents"],
    "Opus-Patent-BSD-3-Clause": ["BSD", "a statement on patents"],
    "Custom license": [], "Ignorable": [], "LicenseRef-takuya-ooura": [], "SPL-SQRT-FLOOR": [],
    "pffft": [],
}

# Inline functions in C header files of the sysroot.
INLINE_FUNCTION = re.compile(
    r"\b(?:static|extern)\s+(?:__always_inline\s+)?(?:__inline__|__inline|inline)\b"
    r"[^;{}()]*?\b([A-Za-z_]\w*)\s*\("
)
LICENCE_DIR = re.compile(r"licen[sc]es?", re.I)
BUNDLED = (".ttf", ".otf", ".woff", ".woff2", ".a", ".lib", ".dll", ".so", ".dylib", ".o", ".obj")
NOT_SHIPPED_DIRS = {"tests", "test", "examples", "example", "benches", "fuzz", "doc", "docs", ".github"}
COPYLEFT = re.compile(r"\b(MPL|LGPL|GPL|AGPL|EPL|CDDL|EUPL)\b")

# What the scan finds below the root of a linked crate (licence files and
# bundled libraries or fonts), looked at by hand at the versions the lock
# file had then:
#   (crate, path prefix): (action, sentence)
# "text" reproduces the files in part 6, "font" adds the font's own name
# table to the sentence, "note" and "skip" print the sentence only. A finding
# without an entry is reported as not looked at, and reproduced if it is text.
REVIEWED = {
    ("regex-syntax", "src/unicode_tables/"): (
        "text",
        "Unicode character tables that are compiled into the crate.",
    ),
    ("tracing-core", "src/spin/"): (
        "text",
        "A copy of the spin crate's lock that is compiled into the crate.",
    ),
    ("wayland-protocols", "protocols/"): (
        "text",
        "The Wayland protocol descriptions the crate's bindings are generated from.",
    ),
    ("wayland-protocols-plasma", "plasma-wayland-protocols/"): (
        "text",
        "KDE's protocol descriptions (plasma-wayland-protocols) the crate's bindings are "
        "generated from at compile time. The crate's Cargo.toml says MIT; the description "
        "files under src/protocols name these licences (in so many files): "
        "{spdx:plasma-wayland-protocols/src/protocols}.",
    ),
    ("libdbus-sys", "vendor/"): (
        "skip",
        "libdbus sources that are only compiled with the crate feature `vendored`. The "
        "feature is off; the application loads the system's libdbus.",
    ),
    ("windows_x86_64_msvc", "lib/"): (
        "skip",
        "The import library for Windows system DLLs that the crate consists of, built by "
        "the windows-rs project that publishes it.",
    ),
    ("webview2-com-sys", "x64/"): (
        "note",
        "Prebuilt loader libraries of Microsoft's WebView2 SDK. For the MSVC target the "
        "crate links x64/WebView2LoaderStatic.lib statically into gelabber-desktop.exe "
        "(src/lib.rs). The crate names no licence for these files and ships no licence "
        "text for them.",
    ),
    ("webview2-com-sys", "x86/"): ("skip", "The same loader for 32-bit x86; not used."),
    ("webview2-com-sys", "arm64/"): ("skip", "The same loader for ARM64; not used."),
    ("sctk-adwaita", "src/title/"): (
        "font",
        "A font that the crate embeds into gelabber-desktop for window titles on Wayland "
        "(include_bytes! in src/title/ab_glyph_renderer.rs, crate feature ab_glyph). The "
        "crate ships no licence text for it.",
    ),
}


# An entry of REVIEWED that only holds while a feature of the crate is off.
FEATURE_MUST_BE_OFF = {"libdbus-sys": "vendored"}


def die(message):
    sys.exit(f"third-party-notices: {message}")


def run(*cmd, cwd=None):
    result = subprocess.run([str(c) for c in cmd], cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        die(f"{' '.join(str(c) for c in cmd)} failed:\n{result.stderr.strip()}")
    return result.stdout


def git(repo, *args):
    # --no-optional-locks: a look at a checkout must not rewrite its index.
    return run("git", "--no-optional-locks", "-C", repo, *args).strip()


def read(path):
    data = Path(path).read_bytes()
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return data.decode("latin-1")


def clean(text):
    """A licence text as it is printed: no CR, form feeds, tabs or trailing blanks."""
    lines = text.replace("\r\n", "\n").replace("\f", "").split("\n")
    return "\n".join(line.expandtabs().rstrip() for line in lines).strip("\n") + "\n"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def fingerprint(*inputs):
    return sha256(json.dumps(inputs, sort_keys=True).encode())[:16]


def para(text, indent=0, bullet=""):
    """A paragraph of the file's own prose; paths and names stay in one piece."""
    return textwrap.fill(
        " ".join(text.split()), WIDTH, break_long_words=False, break_on_hyphens=False,
        initial_indent=" " * indent + bullet, subsequent_indent=" " * (indent + len(bullet)),
    ) + "\n"


def item(text):
    return para(text, 2, "- ")


def listing(items, indent, separator=" "):
    """Items on as few lines as fit, never broken inside an item."""
    lines = [""]
    for index, entry in enumerate(items):
        entry += separator.rstrip() if index + 1 < len(items) else ""
        joined = f"{lines[-1]} {entry}" if lines[-1] else entry
        if lines[-1] and indent + len(joined) > WIDTH:
            lines.append(entry)
        else:
            lines[-1] = joined
    return "".join(f"{' ' * indent}{line}\n" for line in lines)


def text_block(heading_lines, text):
    rule = "-" * WIDTH + "\n"
    return rule + "".join(f"{line}\n" for line in heading_lines) + rule + clean(text) + "\n"


# --- pins and build configuration, read from the repository -----------------


def pins():
    found = {}
    for line in read(NATIVE / "libwebrtc.env").splitlines():
        match = re.match(r"([A-Z_]+)=(.*)$", line)
        if match:
            found[match[1]] = match[2]
    return found


def cmake_archives():
    """FetchContent archives of the core: name -> (url, sha256, file name)."""
    cmake = read(NATIVE / "CMakeLists.txt")
    found = {}
    for name, body in re.findall(r"FetchContent_Declare\(\s*(\w+)\s+([^)]*)\)", cmake):
        url = re.search(r"\bURL (\S+)", body)
        digest = re.search(r"URL_HASH SHA256=(\w+)", body)
        if url and digest:
            file_name = re.search(r"DOWNLOAD_NAME (\S+)", body)
            found[name] = (url[1], digest[1], file_name[1] if file_name else url[1].rsplit("/", 1)[1])
    return found


def linux_build():
    """GN arguments and ninja targets of build-libwebrtc-linux.sh."""
    script = read(NATIVE / "scripts/build-libwebrtc-linux.sh")
    block = re.search(r"^gn_args=\(\n(.*?)^\)", script, re.S | re.M)
    ninja = re.search(r'^ninja -C "\$out" ((?:[^\\\n]|\\\n)*)$', script, re.M)
    if not block or not ninja:
        die("build-libwebrtc-linux.sh: gn_args or the ninja line not found")
    gn_args = [
        line.strip().strip("'")
        for line in block[1].splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]
    return gn_args, ninja[1].replace("\\\n", " ").split()


def patches():
    return sorted(NATIVE.glob("patches/*.patch"))


def build_facts():
    gn_args, _ = linux_build()
    windows = read(NATIVE / "scripts/build-libwebrtc-windows.ps1")
    cmake = [
        line for line in read(NATIVE / "CMakeLists.txt").splitlines()
        if not line.strip().startswith("#")
    ]
    h264 = [line for line in cmake if "WEBRTC_USE_H264" in line]
    bundle = json.loads(read(REPO / "desktop/app/tauri.bundle.windows.json"))["bundle"]
    return {
        "Linux libwebrtc is built with rtc_use_h264=true and FFmpeg's Chrome branding": (
            "rtc_use_h264=true" in gn_args and 'ffmpeg_branding="Chrome"' in gn_args
        ),
        "Windows libwebrtc is built with rtc_use_h264=true": "'rtc_use_h264=true'" in windows,
        "Windows libwebrtc is built with use_custom_libcxx=false": "'use_custom_libcxx=false'" in windows,
        "the core defines WEBRTC_USE_H264 on Linux only": len(h264) == 1 and "WEBRTC_LINUX" in h264[0],
        "the core links the static MSVC runtime on Windows": any(
            'CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded"' in line for line in cmake
        ),
        "the Windows installer embeds the WebView2 bootstrapper": (
            bundle["targets"] == ["nsis"]
            and bundle["windows"]["webviewInstallMode"]["type"] == "embedBootstrapper"
        ),
    }


# --- part 2: libwebrtc -------------------------------------------------------


def libwebrtc_fingerprint():
    return fingerprint(
        LIBWEBRTC_PART, pins()["WEBRTC_COMMIT"], linux_build(),
        [[p.name, sha256(p.read_bytes())] for p in patches()], build_facts(),
    )


def chromium_readme(src, licence_file):
    """Header fields of the README.chromium next to or above a licence file."""
    folder = (src / licence_file).parent
    while folder != src.parent:
        readme = folder / "README.chromium"
        if readme.is_file():
            fields = {}
            for line in read(readme).strip().splitlines():
                if not line.strip():
                    break
                match = re.match(r"([A-Z][A-Za-z ]+):\s*(.*)$", line)
                if match:
                    fields.setdefault(match[1], match[2].strip())
            return readme, fields
        folder = folder.parent
    return None, {}


def readme_licence_files(src, readme, fields):
    """The files a README.chromium names as "License File", as paths in the tree."""
    return [
        (readme.parent / name.strip()).relative_to(src).as_posix()
        for name in fields.get("License File", "").split(",") if name.strip()
    ]


def run_webrtc_generator(src, out_dir, targets):
    """libwebrtc's own generate_licenses.py: [(library, [licence files])], notes."""
    sys.dont_write_bytecode = True
    os.environ["PYTHONDONTWRITEBYTECODE"] = "1"
    spec = importlib.util.spec_from_file_location(
        "webrtc_generate_licenses", src / "tools_webrtc/libs/generate_licenses.py"
    )
    generator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(generator)
    generator.logging.disable(generator.logging.CRITICAL)

    table = dict(generator.LIB_TO_LICENSES_DICT)
    notes = []
    with tempfile.TemporaryDirectory() as scratch:
        # GN runs its helper scripts with "vpython3" (depot_tools); plain
        # python3 does for the ones `gn desc` needs.
        shim = Path(scratch) / "bin/vpython3"
        shim.parent.mkdir()
        shim.write_text(f'#!/bin/sh\nexec "{sys.executable}" "$@"\n')
        shim.chmod(0o755)
        path = os.environ["PATH"]
        os.environ["PATH"] = f"{shim.parent}{os.pathsep}{path}"
        for attempt in range(2):
            builder = generator.LicenseBuilder([str(out_dir)], targets, lib_to_licenses_dict=table)
            try:
                builder.generate_license_text(scratch)
                break
            except Exception as error:  # the generator raises plain Exception
                missing = re.match(r"Missing licenses for third_party targets: (.*)$", str(error))
                if not missing or attempt:
                    die(f"libwebrtc's generate_licenses.py failed: {error}")
                names = missing[1].split(", ")
                for name in names:
                    readme, fields = chromium_readme(src, f"third_party/{name}/README.chromium")
                    if "License File" not in fields:
                        die(f"no licence file known for third_party/{name}")
                    table[name] = readme_licence_files(src, readme, fields)
                notes.append(
                    f"The generator's table has no entry for {' and '.join(names)}: it stops with "
                    f"\"{error}\". For these libraries it was given the licence file that the "
                    "library's README.chromium names in the field \"License File\"."
                )
        os.environ["PATH"] = path
        with open(Path(scratch) / "LICENSE.md") as produced_file:
            produced = produced_file.read()

    # The generator prints one block per library: its licence files, HTML
    # escaped. Take the names from its headings and make sure the output is
    # exactly those files instead of parsing the texts back out of it.
    names = {name for name in re.findall(r"^# (\S+)\n```\n", produced, re.M) if table.get(name)}
    libraries = [(name, table[name]) for name in ["webrtc"] + sorted(names - {"webrtc"})]
    expected = ""
    for name, files in libraries:
        expected += f"# {name}\n```\n"
        for file in files:
            with open(src / file) as licence_file:
                expected += html.escape(licence_file.read(), quote=True) + "\n"
        expected += "```\n\n"
    if produced != expected:
        die("LICENSE.md of libwebrtc's generator is not the concatenation of its licence files")
    return libraries, notes


def ninja_build(src, out_name):
    """What the build in a GN directory compiled, from ninja's own record:
    object file -> (its source, every file the compile read), as paths in
    the checkout. A generated file stands for the directory it was generated
    for; files outside the checkout and other build products are left out."""
    out_dir = src / out_name
    generated = f"{out_name}/gen/"

    def in_tree(path):
        path = os.path.normpath(os.path.join(out_name, path))
        if path.startswith(generated):
            return path[len(generated):]
        outside = path.startswith((f"{out_name}/", "../")) or os.path.isabs(path)
        return None if outside else path

    # .ninja_deps: the files each compile reported having read.
    data = (out_dir / ".ninja_deps").read_bytes()
    version = struct.unpack_from("<i", data, 12)[0] if data.startswith(b"# ninjadeps\n") else 0
    if version not in (3, 4):
        die(f"{out_dir}/.ninja_deps is not a deps log of ninja that this script knows")
    paths, read_by, position = [], {}, 16
    while position < len(data):
        (size,) = struct.unpack_from("<I", data, position)
        record = data[position + 4:position + 4 + (size & 0x7FFFFFFF)]
        position += 4 + len(record)
        if size >> 31:
            # Output, its time (two fields from version 4 on), the inputs.
            ids = struct.unpack(f"<{len(record) // 4}i", record)
            read_by[paths[ids[0]]] = [paths[i] for i in ids[version - 1:]]
        else:
            paths.append(record[:-4].rstrip(b"\0").decode())
    # The assembler writes dependency files instead.
    for depfile in (out_dir / "obj").rglob("*.o.d"):
        target, _, inputs = read(depfile).replace("\\\n", " ").partition(":")
        read_by.setdefault(target.strip(), []).extend(inputs.split())
    sources = {}
    for ninja in out_dir.rglob("*.ninja"):
        sources.update(re.findall(r"^build (obj/\S+\.o): \w+ (\S+)", read(ninja), re.M))

    objects = {}
    for path in sorted((out_dir / "obj").rglob("*.o")):
        name = path.relative_to(out_dir).as_posix()
        source = in_tree(sources[name]) if name in sources else None
        if source is None:
            die(f"{out_dir}: no source in the checkout found for {name}")
        objects[name] = (source, {in_tree(p) for p in read_by.get(name, [])} - {None} | {source})
    return objects


def nm(files, cwd=None):
    """The symbols files define: file -> {name: nm's type letter}."""
    found = {}
    for start in range(0, len(files), 500):
        cmd = ["nm", "-A", "-P", "--defined-only", *(str(f) for f in files[start:start + 500])]
        try:
            result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
        except FileNotFoundError:
            die("nm (binutils) is needed to look into the object files and the media core")
        if any("no symbols" not in line for line in result.stderr.splitlines()):
            die(f"nm failed:\n{result.stderr.strip()}")
        for line in result.stdout.splitlines():
            file, _, symbol = line.partition(": ")
            name, kind = symbol.split()[:2]
            found.setdefault(file, {})[name] = kind
    return found


def linked_objects(objects, out_dir, core_library):
    """The symbols of the media core, and the object files of libwebrtc's
    build that left one there: a function or variable the object defines
    for other files (not weak, not file-local) is a symbol of the library."""
    if not core_library.is_file():
        die(f"{core_library} not found: --core-build is the CMake build directory of the core")
    library = nm([core_library]).get(str(core_library), {})
    if "t" not in library.values():
        die(f"{core_library} is stripped: its symbol table is needed")
    present = {
        name for name, symbols in nm(sorted(objects), cwd=out_dir).items()
        if any(kind in "BCDGRST" and symbol in library for symbol, kind in symbols.items())
    }
    return library, present


def licence_named(name):
    return bool(LICENCE_NAME.match(name)) and not name.endswith(CODE)


def plain(text):
    """A text without its line breaks and comment marks, to look for wording."""
    return " ".join(" ".join(line.strip(" \t*/#") for line in text.splitlines()).split())


def series(names):
    names = list(names)
    return names[0] if len(names) == 1 else f"{', '.join(names[:-1])} and {names[-1]}"


def compared(library, declared, texts):
    """Where the licence names of a README.chromium and the wording found in
    the printed licence files differ, as sentences."""
    names = [name.strip() for name in declared.split(",") if name.strip()]
    unknown = [name for name in names if name not in NAMED_LICENCES]
    if unknown:
        die(f"README.chromium of {library} names {unknown}: say in NAMED_LICENCES what to look for")
    found = {kind for kind, wording in WORDING.items() if any(re.search(wording, t) for t in texts)}
    missing = [name for name in names if not set(NAMED_LICENCES[name]) <= found]
    named = {kind for name in names for kind in NAMED_LICENCES[name]}
    unnamed = [kind for kind in ALWAYS_LOOKED_FOR if kind in found and kind not in named]
    files = "file" if len(texts) == 1 else "files"
    out = []
    if missing:
        out.append(
            f"README.chromium names {series(missing)}; the wording of "
            f"{'that' if len(missing) == 1 else 'these'} was not found in the library's licence "
            f"{files} printed here.")
    if unnamed:
        out.append(
            f"The licence {files} printed here {'holds' if len(texts) == 1 else 'hold'} the "
            f"wording of {series(unnamed)}, which README.chromium does not name.")
    return " ".join(out)


def build_survey(src, objects, roots):
    """What ninja's record says per library and directory. roots says where
    the libraries live (directory -> name, "" for libwebrtc itself). Returns
    the object files compiled from a library's sources, those of others that
    read one of its files, the object files that read a file in a directory
    or below it, and per library the licence files in those directories."""
    owners = {}

    def owner(folder):
        if folder not in owners:
            owners[folder] = roots[folder] if folder in roots else owner(os.path.dirname(folder))
        return owners[folder]

    own, readers, users = {}, {}, {}
    for name, (source, files) in objects.items():
        source_owner = owner(os.path.dirname(source))
        own.setdefault(source_owner, set()).add(name)
        folders = set()
        for file in files:
            folder = os.path.dirname(file)
            if owner(folder) == roots[""] and re.search(r"(^|/)third_party/", file) \
                    and not file.startswith(COMPILER_DIRECTORY):
                die(f"the build read {file}, which is in no library of the generator's list")
            if owner(folder) != source_owner:
                readers.setdefault(owner(folder), set()).add(name)
            while folder not in folders:
                folders.add(folder)
                folder = os.path.dirname(folder)
        for folder in folders:
            users.setdefault(folder, set()).add(name)

    licence_files = {}
    for folder in sorted(users):
        if (src / folder).is_dir():
            for file in sorted(os.listdir(src / folder)):
                if licence_named(file) and (src / folder / file).is_file():
                    licence_files.setdefault(owner(folder), []).append(os.path.join(folder, file))
    return own, readers, users, licence_files


def library_texts(src, name, generator_files, named, found):
    """The licence files of a library: those of libwebrtc's generator and of
    its README.chromium (named), what stands next to them, and of the files
    the survey found, those of directories below the library. Returns the
    files to print, {reason: [files next to them that are not printed]} and
    the files of directories below."""
    printed = generator_files + [file for file in named if file not in generator_files]
    beside_these = {os.path.dirname(file) for file in printed}
    beside = {
        os.path.join(folder, file) for folder in beside_these for file in os.listdir(src / folder)
        if licence_named(file) and (src / folder / file).is_file()
    } | {
        file for file, verdict in BESIDE_THE_LICENCE.items()
        if verdict is True and os.path.dirname(file) in beside_these
    }
    left_out = {}
    for file in sorted(beside - set(printed)):
        verdict = BESIDE_THE_LICENCE.get(file)
        if verdict is True:
            printed.append(file)
            continue
        if not verdict:
            holder = next((p for p in printed if plain(read(src / file)) in plain(read(src / p))), None)
            if not holder:
                die(f"{file} stands next to the licence file of {name}: add it to BESIDE_THE_LICENCE")
            verdict = f"Its text is part of {holder}, printed here."
        left_out.setdefault(verdict, []).append(os.path.basename(file))
    below = [
        file for file in found if file not in printed and os.path.dirname(file) not in beside_these
    ]
    return printed, left_out, below


def field(label, text):
    """A heading line "  Label     text"; long text continues below its start."""
    lines = textwrap.wrap(text, WIDTH - 12, break_long_words=False, break_on_hyphens=False)
    return [f"  {label:<9} {lines[0]}"] + [" " * 12 + line for line in lines[1:]]


def ffmpeg_configuration(src, gn_args, library):
    branding = next(a.split("=", 1)[1].strip('"') for a in gn_args if a.startswith("ffmpeg_branding="))
    config = Path("third_party/ffmpeg/chromium/config") / branding / "linux/x64"
    header = read(src / config / "config.h")
    licence = re.search(r'#define FFMPEG_LICENSE "(.*)"', header)[1]
    switches = {
        name: re.search(rf"#define CONFIG_{name} (\d)", header)[1] for name in ("GPL", "NONFREE", "VERSION3")
    }
    if licence != "LGPL version 2.1 or later" or set(switches.values()) != {"0"}:
        die(f"{config}/config.h: the sentences on FFmpeg describe an LGPL 2.1 configuration")
    enabled = {}
    components = read(src / config / "config_components.h")
    for name, kind in re.findall(r"#define CONFIG_(\w+)_(DECODER|PARSER|DEMUXER|ENCODER|MUXER) 1", components):
        enabled.setdefault(kind.lower(), []).append(name.lower())
    out = para(
        f"Licence of this build as its configuration states it ({config}/config.h): "
        f"FFMPEG_LICENSE \"{licence}\", CONFIG_GPL {switches['GPL']}, CONFIG_NONFREE "
        f"{switches['NONFREE']}, CONFIG_VERSION3 {switches['VERSION3']}.")
    out += para(
        "Components that configuration enables, which are compiled into the libwebrtc "
        f"package, and which of them are among the symbols of {CORE_LIBRARY} (Linux; FFmpeg "
        "registers a component as ff_<name>_<kind>):")
    for kind in sorted(enabled):
        names = sorted(enabled[kind])
        linked = [name for name in names if f"ff_{name}_{kind}" in library]
        which = "none" if not linked else "all" if linked == names else series(linked)
        out += para(f"{kind}s ({len(names)}; in {CORE_LIBRARY}: {which}):", 2) + listing(names, 4)
    return out


def leading_comment(text):
    comment = re.match(r"\s*/\*(.*?)\*/", text, re.S)
    lines = [re.sub(r"^\s*\*? ?", "", line).rstrip() for line in comment[1].splitlines()] if comment else []
    return "\n".join(lines).strip("\n")


def system_headers(src, library):
    """Inline functions that C header files of the sysroot define and that
    are file-local functions of the media core, by name; with the notice
    each such header begins with."""
    sysroots = sorted((src / "build/linux").glob("debian_*amd64-sysroot"))
    if len(sysroots) != 1:
        die(f"{src}/build/linux: expected one amd64 sysroot, found {len(sysroots)}")
    sysroot = sysroots[0]
    local = set()
    for name, kind in library.items():
        if kind == "t":
            mangled = re.match(r"_ZL(\d+)", name)
            if mangled:
                local.add(name[mangled.end():mangled.end() + int(mangled[1])])
            elif not name.startswith("_Z"):
                local.add(name)
    notices = {}  # notice -> [(header, functions)]
    for folder, folders, files in os.walk(sysroot / "usr/include"):
        folders.sort()
        if "c++" in folders and Path(folder) == sysroot / "usr/include":
            folders.remove("c++")
        for file in sorted(files):
            if not (Path(folder) / file).is_file():
                continue
            text = read(Path(folder) / file)
            names = sorted(set(INLINE_FUNCTION.findall(text)) & local) if "inline" in text else []
            if names:
                header = (Path(folder) / file).relative_to(sysroot).as_posix()
                notices.setdefault(leading_comment(text), []).append((header, names))

    stamp = sysroot / ".stamp"
    heading = ["Header files of system libraries"]
    heading += field("From", f"{sysroot.relative_to(src).as_posix()} in libwebrtc's tree")
    if stamp.is_file():
        heading += field("", read(stamp).strip())
    body = para(
        "libwebrtc and the media core are compiled for Linux against this sysroot: header "
        "files of Debian packages for the system libraries the programs load when they run. "
        "A function that such a header defines inline is compiled into the code that uses "
        f"it. Looked for by name: the file-local functions among the symbols of {CORE_LIBRARY} "
        "that a C header of the sysroot defines as an inline function (the headers of the "
        "C++ standard library left out). Inline code that the compiler merged into its "
        "callers, macros and templates leave no symbol and are not found this way; nothing "
        "else was collected for system header files.")
    if not notices:
        body += "\n" + para("No such function was found.")
    for notice, headers in notices.items():
        body += "\n"
        for header, names in headers:
            body += f"[{header}]\n" + para(f"Functions: {', '.join(names)}.")
        body += "\n" + (clean(notice) if notice else para("The file does not begin with a notice."))
    return text_block(heading, body)


def libwebrtc_part(src, out_name, core_build):
    src = Path(src).resolve()
    pin = pins()
    if git(src, "rev-parse", "HEAD") != pin["WEBRTC_COMMIT"]:
        die(f"{src} is not at WEBRTC_COMMIT {pin['WEBRTC_COMMIT']}")
    patched = sorted(
        path for p in patches() for path in re.findall(r"^\+\+\+ b/(\S+)", read(p), re.M)
    )
    changed = sorted(
        line.split()[-1]
        for line in git(src, "status", "--porcelain", "--untracked-files=no").splitlines()
    )
    if changed != patched:
        die(f"{src} differs from the pinned commit in {changed}, the patches change {patched}")
    gn_args, targets = linux_build()
    out_dir = src / out_name
    built_with = [line.replace(" = ", "=") for line in read(out_dir / "args.gn").splitlines() if line.strip()]
    if built_with != gn_args:
        die(f"{out_dir}/args.gn is not what build-libwebrtc-linux.sh passes")

    libraries, notes = run_webrtc_generator(src, out_dir, targets)
    names = [name for name, _ in libraries]
    if not {"libc++", "libc++abi", "llvm-libc", "ffmpeg", "openh264"} <= set(names):
        die("the Windows paragraph of part 2 names libraries that are no longer in the list")

    # Where each library lives in the tree: the directory of its
    # README.chromium, else that of its licence file; the rest is libwebrtc.
    readmes, roots = {}, {"": "webrtc"}
    for name, files in libraries:
        readme, fields = chromium_readme(src, files[0])
        if name != "webrtc" and readme and readme.parent == src:
            readme, fields = None, {}
        readmes[name] = (readme, fields)
        if name != "webrtc":
            root = (readme.parent if readme else (src / files[0]).parent).relative_to(src).as_posix()
            if roots.setdefault(root, name) != name:
                die(f"{name} and {roots[root]} share the directory {root}")

    # What the build compiled, and what of it is in the Linux library.
    objects = ninja_build(src, out_name)
    library, present = linked_objects(objects, out_dir, Path(core_build).resolve() / CORE_LIBRARY)
    own, readers, users, licence_files = build_survey(src, objects, roots)
    if not own.get("webrtc", set()) & present:
        die(f"{CORE_LIBRARY} has no symbol of libwebrtc's object files: not built against {out_dir}?")
    for name in ("ffmpeg", "openh264"):
        if not own.get(name, set()) & present:
            die(f"parts 1 and 2 say that {name} is in {CORE_LIBRARY}, but no object file of it is")

    def on_linux(name):
        mine, others = own.get(name, set()), readers.get(name, set())
        line = (
            f"in {CORE_LIBRARY}: {len(mine & present)} of its {len(mine)} object files"
            if mine else "no object files of its own in the build"
        )
        if others:
            line += (", and " if mine else f"; in {CORE_LIBRARY}: ")
            line += f"{len(others & present)} of the {len(others)} other object files that read its files"
        return line

    out = para(
        "libwebrtc is built from source at the pinned commit and linked statically into the "
        "native media core, together with the third-party libraries of its source tree that "
        "the build uses.")
    out += "\n"
    out += f"  Source    {git(src, 'config', '--get', 'remote.origin.url')}\n"
    out += f"  Commit    {pin['WEBRTC_COMMIT']} ({pin['WEBRTC_BRANCH']})\n"
    for index, patch in enumerate(patches()):
        out += f"  {'Changed   ' if index == 0 else '          '}{patch.relative_to(REPO).as_posix()}\n"
    out += "  Build     desktop/native/scripts/build-libwebrtc-linux.sh, GN arguments:\n"
    out += listing(gn_args, 12)
    out += "\nHow the list was made\n"
    out += para(
        "With libwebrtc's own tools_webrtc/libs/generate_licenses.py at that commit, run "
        f"against the Linux build directory ({out_name}) for the GN targets the build script "
        f"builds ({' '.join(targets)}). The generator lists the third-party libraries those "
        "targets depend on and prints, for each, the licence files its table names.", 2)
    for note in notes:
        out += para(note, 2)
    out += para(
        "The generator's table names one licence file per library. Added from the same "
        "checkout and build directory:", 2)
    out += item(
        "the files that a library's README.chromium names as \"License File\" and the "
        "generator's table lacks;")
    out += item(
        "licence files of directories below a library. ninja's record of the build (its "
        "dependency log, the assembler's dependency files and the build statements) says "
        "which files every compile read. A file named like a licence file that stands in "
        "the directory of such a file, or in a directory above it, and is neither one of "
        "the library's own licence files nor next to them, is printed under the library it "
        "belongs to, with what the build read from that directory;")
    out += item(
        "files next to a library's licence file that neither the generator nor "
        "README.chromium names. They were looked at one by one (the table "
        "BESIDE_THE_LICENCE of the script); an entry says which of them it does not print "
        "and why.")
    out += para(
        "The same record was used to check the list against the build: every object file "
        "was compiled from a file of libwebrtc's own checkouts or of a listed library, and "
        "no file a compile read lies in a third_party directory outside the listed "
        f"libraries, apart from the compiler's own headers ({COMPILER_DIRECTORY.rstrip('/')}).", 2)
    out += para(
        "Not collected: notices that stand only at the head of individual source files and "
        "not in a licence file.", 2)
    out += para(
        "\"From\" is the git repository and commit the licence file was checked out from; "
        "apart from the change named above, none of these checkouts differed from its "
        "commit. \"README\" quotes the README.chromium that libwebrtc's tree keeps for the "
        "library. Those are statements of that file and were not verified, with one "
        "exception: the script looks in the library's printed licence files for the wording "
        "of each licence README.chromium names, and in every case for the wording of "
        f"{series(ALWAYS_LOOKED_FOR)}. Where the two differ, the entry says so. That is a "
        "search for wording, not a reading of the texts.", 2)
    out += "\nWhat is in the Linux library\n"
    out += para(
        "The generator's list is a list of dependencies. It can name more than the binary "
        "holds: the linker leaves out object files that nothing refers to, and tools of the "
        f"build are dependencies too. The lines \"Linux\" say what was found in a {CORE_LIBRARY} "
        "with its symbol table, built from the pins of parts 2 and 3 on the machine that "
        "generated this part. An object file of libwebrtc's build counts as being in the "
        "library when a function or variable that it defines for other files (not weak, not "
        "file-local) is among the library's symbols. \"Other object files that read its "
        "files\" were compiled with a file of the library, usually a header, without "
        "belonging to it; whether code from that file reached them is not visible. The "
        "library in the packages is built separately from the same sources and was not "
        "looked into.", 2)
    out += "\nWindows\n"
    out += para(
        "No list was generated for the Windows build of libwebrtc (target_os=\"win\", "
        "desktop/native/scripts/build-libwebrtc-windows.ps1); the list for Linux stands in "
        "for it. Differences known from that script and from desktop/native/CMakeLists.txt:", 2)
    out += item(
        "use_custom_libcxx=false: libc++, libc++abi and llvm-libc are not part of the "
        "Windows build, which uses Microsoft's C++ standard library and C runtime instead "
        "(see part 1).")
    out += item(
        "ffmpeg and openh264 are compiled into the Windows libwebrtc package as well "
        "(rtc_use_h264=true), but the media core is built without WEBRTC_USE_H264 on "
        "Windows, so its own code refers to neither (see part 1).")
    out += para(
        "Whether the Windows build depends on third-party libraries that the Linux build "
        "lacks was not established. gelabber_media.dll was not looked into: the lines "
        "\"Linux\" and the entry on system header files describe the Linux library only.", 2)
    out += "\nLibraries\n"
    out += listing(names, 2)
    out += para("After them: header files of system libraries.", 2)

    for name, generator_files in libraries:
        readme, fields = readmes[name]
        folder = (src / generator_files[0]).parent
        top = Path(git(folder, "rev-parse", "--show-toplevel"))
        if top != src and git(top, "status", "--porcelain", "--untracked-files=no"):
            die(f"{top} has local changes")
        heading = [
            name,
            f"  From      {git(top, 'config', '--get', 'remote.origin.url')}",
            f"            commit {git(top, 'rev-parse', 'HEAD')}",
        ]
        # Code that gclient checks out below the library's directory, apart
        # from the checkout the licence file is in (dav1d).
        for nested in sorted((readme.parent if readme and name != "webrtc" else folder).glob("*/.git")):
            if nested.parent != top:
                heading.append(f"  Code      {git(nested.parent, 'config', '--get', 'remote.origin.url')}")
                heading.append(f"            commit {git(nested.parent, 'rev-parse', 'HEAD')}")
        quoted = [
            f"{key}: {fields[key]}"
            for key in (("License",) if name == "webrtc" else ("Name", "Version", "Revision", "License", "Shipped"))
            if fields.get(key, "N/A") not in ("N/A", "DEPS")
        ]
        if quoted:
            heading += field("README", f"{readme.relative_to(src).as_posix()} says")
            for line in quoted:
                heading += field("", line)

        named = readme_licence_files(src, readme, fields) if readme else []
        printed, left_out, below = library_texts(
            src, name, generator_files, named, licence_files.get(name, []))
        files = printed + below
        for index, file in enumerate(files):
            heading.append(f"  {'Text' if index == 0 else '':<9} {file}")
        heading += field("Linux", on_linux(name))

        body = ""
        if name == "ffmpeg":
            body += para(
                f"FFmpeg is linked statically, as part of libwebrtc, into {CORE_LIBRARY} "
                "(Linux). It is Chromium's copy of FFmpeg, unchanged. The source code is the "
                "repository and commit named above; the commands that build it into the media "
                "core are desktop/native/scripts/build-libwebrtc-linux.sh "
                f"and desktop/native/CMakeLists.txt in the Gelabber repository ({GELABBER_URL}) "
                "at the tag of the release.")
            body += ffmpeg_configuration(src, gn_args, library) + "\n"
        if name == "openh264":
            body += para(
                f"OpenH264 is linked statically, as part of libwebrtc, into {CORE_LIBRARY} "
                "(Linux), built from the source named above. The README.chromium of libwebrtc's "
                "tree notes for it that licences other than the BSD licence of the source apply "
                "to builds (MPEG LA patents) and refers to www.openh264.org.") + "\n"
        difference = compared(name, fields.get("License", ""), [plain(read(src / f)) for f in printed])
        if difference:
            body += para(difference) + "\n"
        for reason, others in left_out.items():
            body += para(f"Not printed: {series(others)}, next to the licence file. {reason}") + "\n"
        described = set()
        for file in files:
            if len(files) > 1:
                body += f"[{file}]\n\n"
            directory = os.path.dirname(file)
            if file in below and directory not in described:
                described.add(directory)
                read_there = sorted({
                    f[len(directory) + 1:] for user in users[directory] for f in objects[user][1]
                    if f.startswith(directory + "/")
                })
                which = f" ({', '.join(read_there)})" if len(read_there) <= 8 else ""
                body += para(
                    f"A directory below {name} with licence files of its own. The build read "
                    f"{len(read_there)} of its files{which}. In {CORE_LIBRARY} (Linux): "
                    f"{len(users[directory] & present)} of the {len(users[directory])} object "
                    "files compiled with them.") + "\n"
            body += clean(read(src / file)) + "\n"
        out += "\n" + text_block(heading, body)
    return out + "\n" + system_headers(src, library)


# --- part 3: the other libraries of the media core ---------------------------


def core_fingerprint():
    pin = pins()
    return fingerprint(
        CORE_PART, pin["LIBMEDIASOUPCLIENT_COMMIT"], pin["LIBSDPTRANSFORM_TAG"], cmake_archives()
    )


def fetched_archive(deps, name, archives):
    url, digest, file_name = archives[name]
    path = deps / f"{name}-subbuild/{name}-populate-prefix/src/{file_name}"
    if not path.is_file() or sha256(path.read_bytes()) != digest:
        die(f"{path} is not the archive pinned in desktop/native/CMakeLists.txt ({digest})")
    return url, digest, tarfile.open(path)


def member(archive, name):
    """A file of an archive, by its path with or without the top directory."""
    for info in archive.getmembers():
        if info.isfile() and name in (info.name, info.name.split("/", 1)[-1]):
            return archive.extractfile(info).read().decode("utf-8")
    die(f"{name} not found in {archive.name}")


def core_part(build_dir):
    deps = Path(build_dir).resolve() / "_deps"
    pin = pins()
    archives = cmake_archives()
    cmake = read(NATIVE / "CMakeLists.txt")

    client = deps / "mediasoupclient-src"
    if git(client, "rev-parse", "HEAD") != pin["LIBMEDIASOUPCLIENT_COMMIT"]:
        die(f"{client} is not at LIBMEDIASOUPCLIENT_COMMIT")
    client_url = re.search(r"GIT_REPOSITORY (\S*libmediasoupclient\S*)", cmake)[1]
    client_cmake = read(client / "CMakeLists.txt")
    sdp_url = re.search(r"GIT_REPOSITORY (\S*libsdptransform\S*)", client_cmake)[1]
    sdp_tag = re.search(r"GIT_REPOSITORY \S*libsdptransform\S*\s+GIT_TAG (\S+)", client_cmake)[1]
    sdp = deps / "libsdptransform-src"
    if sdp_tag != pin["LIBSDPTRANSFORM_TAG"] or git(sdp, "describe", "--tags", "--exact-match") != sdp_tag:
        die(f"{sdp} is not libsdptransform {pin['LIBSDPTRANSFORM_TAG']}")
    json_header = read(sdp / "include/json.hpp")
    json_version = re.search(r"JSON for Modern C\+\+\n.*?version (\S+)", json_header)[1]
    json_notices = list(dict.fromkeys(
        line.strip("/*@ ").removeprefix("copyright ")
        for line in json_header.splitlines()
        if re.search(r"SPDX-|Copyright \(c\)|licensed under", line)
    ))

    rnnoise_url, rnnoise_digest, rnnoise = fetched_archive(deps, "rnnoise", archives)
    model_url, model_digest, model = fetched_archive(deps, "rnnoise_model", archives)
    model_version = member(rnnoise, "model_version").strip()
    if model_version not in model_url:
        die(f"RNNoise names model {model_version}, CMakeLists.txt fetches {model_url}")
    model_files = sorted(info.name for info in model.getmembers() if info.isfile())
    model_sources = [member(model, name) for name in model_files if name.endswith((".c", ".h"))]
    if any(LICENCE_NAME.match(Path(name).name) for name in model_files) or any(
        re.search(r"copyright|licen[sc]e", text[:4000], re.I) for text in model_sources
    ):
        die("the RNNoise model archive now carries a licence statement: print it")
    if "https://media.xiph.org/rnnoise/models/" not in member(rnnoise, "download_model.sh") or \
            not model_url.startswith("https://media.xiph.org/rnnoise/models/"):
        die("the RNNoise model no longer comes from the address RNNoise's download_model.sh uses")

    def checked(text, *marks):
        if not all(mark in text for mark in marks):
            die(f"a licence text is not the expected one (looked for {marks})")
        return text

    out = para(
        "Linked statically into the native media core on both systems, built from source by "
        "desktop/native/CMakeLists.txt at the versions pinned there and in "
        "desktop/native/libwebrtc.env.")
    out += "\n"
    out += text_block(
        [
            "libmediasoupclient",
            f"  From      {client_url}",
            f"            commit {pin['LIBMEDIASOUPCLIENT_COMMIT']}",
            "  Licence   ISC",
            "  Text      LICENSE",
        ],
        checked(read(client / "LICENSE"), "ISC License", "Permission to use, copy, modify"),
    )
    out += text_block(
        [
            f"libsdptransform {sdp_tag}",
            f"  From      {sdp_url}",
            f"            tag {sdp_tag}, commit {git(sdp, 'rev-parse', 'HEAD')}",
            "            (fetched by libmediasoupclient's CMakeLists.txt)",
            "  Licence   MIT",
            "  Text      LICENSE",
        ],
        checked(read(sdp / "LICENSE"), "MIT License", "Permission is hereby granted"),
    )
    out += "-" * WIDTH + "\n"
    out += f"JSON for Modern C++ (nlohmann/json) {json_version}\n"
    out += f"  From      the single header include/json.hpp inside libsdptransform {sdp_tag}\n"
    out += "  Licence   named in the header by identifier only, see below\n"
    out += "-" * WIDTH + "\n"
    out += para(
        "libsdptransform's tree holds no licence text for this header. The licence and "
        "copyright statements in the header itself are:")
    out += "".join(item(line) for line in json_notices) + "\n"
    out += text_block(
        [
            "RNNoise",
            f"  From      {rnnoise_url}",
            f"            sha256 {rnnoise_digest}",
            "  Licence   BSD-3-Clause",
            "  Text      COPYING",
        ],
        checked(member(rnnoise, "COPYING"), "Redistribution and use", "Neither the name"),
    )
    out += "-" * WIDTH + "\n"
    out += f"RNNoise model {model_version}\n"
    out += f"  From      {model_url}\n"
    out += f"            sha256 {model_digest}\n"
    out += "  Licence   not stated in the archive, see below\n"
    out += "-" * WIDTH + "\n"
    out += para(
        f"The weights of the noise suppression network ({', '.join(model_files)}; "
        "rnnoise_data.c is compiled in). It is the model that the RNNoise source above "
        "names in its file model_version and that its download_model.sh fetches from the "
        "same address. The archive carries no licence file and its files no licence "
        "header.")
    return out


# --- parts 4 to 6: the application -------------------------------------------


def rust_toolchain():
    """Channel and directory of the toolchain that rust-toolchain.toml pins.
    cargo works out what build scripts and macros depend on for the machine
    it runs on, so the file is defined as the one a Linux x86-64 host writes."""
    channel = re.search(r'channel\s*=\s*"([^"]+)"', read(REPO / "rust-toolchain.toml"))[1]
    version = run("rustc", "-vV", cwd=REPO)
    host, release = (re.search(rf"^{key}: (\S+)", version, re.M)[1] for key in ("host", "release"))
    if host != HOST:
        die(f"run this on {HOST}: on {host} cargo reports other build-time crates")
    if release != channel:
        die(f"rustc is {release}, rust-toolchain.toml pins {channel}: run it through rustup")
    return channel, Path(run("rustc", "--print", "sysroot", cwd=REPO).strip())


def cargo(*args, offline):
    cmd = ["cargo", *args, "--locked", "--manifest-path", REPO / "desktop/Cargo.toml"]
    return run(*cmd, *(["--offline"] if offline else []), cwd=REPO)


def cargo_tree(target, edges, offline):
    out = cargo(
        "tree", "-p", "gelabber-desktop", "--target", target, "--prefix", "none",
        "--format", "{p}", "-e", edges, offline=offline,
    )
    return {match.groups() for match in re.finditer(r"^(\S+) v(\S+)", out, re.M)}


def font_names(path):
    """Name, version, copyright and licence address from a font's name table."""
    data = Path(path).read_bytes()
    wanted = {4: "name", 5: "version", 0: "copyright", 14: "licence address"}
    found = {}
    for index in range(struct.unpack(">H", data[4:6])[0]):
        tag, _, offset, _ = struct.unpack(">4sIII", data[12 + 16 * index:28 + 16 * index])
        if tag != b"name":
            continue
        _, count, strings = struct.unpack(">HHH", data[offset:offset + 6])
        for record in range(count):
            platform, _, _, name, length, start = struct.unpack(
                ">6H", data[offset + 6 + 12 * record:offset + 18 + 12 * record])
            raw = data[offset + strings + start:offset + strings + start + length]
            if name in wanted and wanted[name] not in found:
                text = raw.decode("utf-16-be" if platform in (0, 3) else "latin-1")
                found[wanted[name]] = " ".join(text.split())
    return "; ".join(f"{field} \"{found[field]}\"" for field in wanted.values() if field in found)


def scan_crate(root):
    """Licence files in a crate's root, and what lies below it: licence
    files and bundled libraries or fonts outside tests and examples."""
    top, below = [], []
    for folder, folders, files in os.walk(root):
        rel = Path(folder).relative_to(root)
        if rel.parts and rel.parts[0].lower() in NOT_SHIPPED_DIRS:
            folders[:] = []
            continue
        folders.sort()
        in_licence_dir = any(LICENCE_DIR.fullmatch(part) for part in rel.parts)
        for name in sorted(files):
            if not rel.parts:
                if LICENCE_NAME.match(name):
                    top.append(name)
            elif LICENCE_NAME.match(name) or in_licence_dir or name.lower().endswith(BUNDLED):
                below.append((rel / name).as_posix())
    return top, below


def spdx_census(folder):
    counts = {}
    for path in sorted(Path(folder).rglob("*")):
        if path.is_file():
            for name in set(re.findall(r"SPDX-License-Identifier:\s*([^\s<]+)", read(path))):
                counts[name] = counts.get(name, 0) + 1
    return ", ".join(f"{name} in {count}" for name, count in sorted(counts.items()))


def below_root(name, root, below, features):
    """What REVIEWED says about a crate's findings: files to reproduce, remarks."""
    wanted, remarks = [], {}
    for path in below:
        action, sentence = next(
            (
                entry for (crate, prefix), entry in REVIEWED.items()
                if crate == name and path.startswith(prefix)
            ),
            (None, None),
        )
        if action is None or FEATURE_MUST_BE_OFF.get(name) in features:
            action = "bundled" if path.lower().endswith(BUNDLED) else "text"
            sentence = "Not looked at yet" + ("." if action == "bundled" else "; reproduced in part 6.")
        if action == "text":
            wanted.append(path)
        if action == "font":
            sentence += f" The font's own name table: {font_names(root / path)}."
        sentence = re.sub(r"\{spdx:(.*?)\}", lambda found: spdx_census(root / found[1]), sentence)
        remarks.setdefault(sentence, []).append(path)
    return wanted, [f"{', '.join(paths)}. {sentence}" for sentence, paths in remarks.items()]


def rust_parts(offline):
    packages, features, linked, build_time = {}, {}, {}, set()
    graphs, reported = {}, {}
    for target, system in TARGETS.items():
        # Per target: without the filter cargo wants every crate of the lock
        # file, also those of systems the app is not built for.
        metadata = json.loads(
            cargo("metadata", "--format-version", "1", "--filter-platform", target, offline=offline)
        )
        for package in metadata["packages"]:
            known = packages.setdefault((package["name"], package["version"]), package)
            if known["id"] != package["id"]:
                die(f"two packages are called {package['name']} {package['version']}")
        graphs[target] = {node["id"]: node for node in metadata["resolve"]["nodes"]}
        for node in metadata["resolve"]["nodes"]:
            features.setdefault(node["id"], set()).update(node["features"])
        in_binary = cargo_tree(target, "normal,no-proc-macro", offline)
        for key in in_binary:
            linked.setdefault(key, []).append(system)
        reported[target] = cargo_tree(target, "normal,build", offline)
        build_time |= reported[target]

    # cargo works out what build scripts and macros depend on for the machine
    # it runs on, here Linux. What a build on Windows adds: the dependencies
    # for Windows that the manifest of a reported crate names as not
    # optional, and theirs in turn.
    windows = next(target for target, system in TARGETS.items() if system == "Windows")
    by_id = {package["id"]: key for key, package in packages.items()}
    on_windows, queue = set(), [packages[key]["id"] for key in reported[windows]]
    while queue:
        node = graphs[windows][queue.pop()]
        required = {
            dependency["name"] for dependency in packages[by_id[node["id"]]]["dependencies"]
            if dependency["kind"] in (None, "build") and not dependency["optional"]
        }
        for dependency in node["deps"]:
            key = by_id[dependency["pkg"]]
            if key[0] in required and key not in reported[windows] | on_windows \
                    and any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"]):
                on_windows.add(key)
                queue.append(dependency["pkg"])

    def third_party(keys):
        # Without a source it is a crate of this workspace.
        return sorted(key for key in keys if packages[key]["source"])

    texts = {}  # text without its whitespace -> [number, text, crates, file names]
    lines, without_text, source_terms, remarks = [], [], [], []
    for key in third_party(linked):
        package = packages[key]
        label = f"{key[0]} {key[1]}"
        root = Path(package["manifest_path"]).parent
        top, below = scan_crate(root)
        if package["license_file"] and package["license_file"] not in top:
            top.append(package["license_file"])
        nested, said = below_root(key[0], root, below, features[package["id"]])
        remarks += [f"{label}: {remark}" for remark in said]
        numbers = []
        for file in top + nested:
            text = clean(read(root / file))
            entry = texts.setdefault("".join(text.split()), [f"R{len(texts) + 1:03}", text, [], []])
            entry[2].append(label)
            entry[3].append(Path(file).name)
            numbers.append(entry[0])
        expression = package["license"] or f"see {package['license_file']}"
        from_registry = package["source"].startswith("registry+https://github.com/rust-lang/crates.io-index")
        source = f"https://crates.io/crates/{key[0]}/{key[1]}" if from_registry else package["source"]
        lines.append(" | ".join([
            label, ", ".join(linked[key]), expression,
            ", ".join(dict.fromkeys(numbers)) or "no licence file",
            *([] if from_registry else [f"from {source}"]),
        ]))
        if not top:
            without_text.append(
                f"{label}: licence \"{expression}\"; authors: "
                f"{', '.join(package['authors']) or 'not stated'}; repository: "
                f"{package['repository'] or 'not stated'}")
        if COPYLEFT.search(expression):
            source_terms.append(f"{label}: {expression}; source: {source}")

    crates = para(
        "The crates cargo links into gelabber-desktop (Linux) and gelabber-desktop.exe "
        "(Windows) according to desktop/Cargo.lock, as cargo itself reports them for "
        f"{' and '.join(TARGETS)}:")
    crates += "\n    cargo tree -p gelabber-desktop -e normal,no-proc-macro --target <target>\n\n"
    crates += para(
        "Licence expressions are the ones in each crate's Cargo.toml (cargo metadata). "
        "Texts are the licence files in the root directory of the crate's published "
        "source, plus the files named under \"Below the root\" further down; part 6 "
        "prints each distinct text once. Unless a line says otherwise, a crate comes from "
        "crates.io at the version shown, where its source is published as "
        "https://crates.io/crates/<name>/<version>.")
    crates += para(
        "Not collected: notices that stand only inside individual source files, for "
        "example in the Wayland protocol descriptions that some crates turn into code.")
    crates += f"\n{len(lines)} crates: name version | linked on | licence | texts\n\n"
    crates += "".join(f"{line}\n" for line in lines)

    def section(title, intro, entries, none):
        if not entries:
            return f"\n{title}\n  {none}\n"
        return f"\n{title}\n" + para(intro, 2) + "".join(item(entry) for entry in entries)

    crates += section(
        "Crates without a licence text",
        "The published source of these crates has no licence file in its root. What their "
        "Cargo.toml states instead:",
        without_text, "None.")
    crates += section(
        "Crates under MPL, LGPL, GPL, AGPL, EPL, CDDL or EUPL",
        "Found by licence expression, with the address of the source code they are compiled "
        "from:",
        source_terms, "None by licence expression.")
    crates += section(
        "Below the root",
        "Licence files, prebuilt libraries and fonts that lie below the root of a linked "
        "crate, outside its tests and examples:",
        remarks, "Nothing found.")

    def named(keys):
        return "".join(
            f"  {key[0]} {key[1]} | "
            f"{packages[key]['license'] or 'see ' + str(packages[key]['license_file'])}\n"
            for key in third_party(keys)
        )

    crates += "\nBuild time only\n"
    crates += para(
        "Procedural macros, build scripts and what those depend on. They run on the "
        "machine that builds the application; their own code is not linked into it, the "
        "code they generate is. cargo works out this group for the machine it runs on: "
        "the list is what cargo reports on Linux x86-64 for the two targets (-e "
        "normal,build, without the crates listed above). No texts are reproduced for "
        "them.", 2)
    crates += "\n" + named(build_time - set(linked)) + "\n"
    crates += para(
        "The Windows package is built on Windows. For that machine the manifests of the "
        "crates cargo reported name further dependencies as not optional, which cargo on "
        "Linux leaves out. These and what they depend on in turn, taken from cargo "
        "metadata and not from a build:", 2)
    crates += "\n" + (named(on_windows - set(linked)) or "  None.\n")

    licence_texts = para(
        "Each distinct licence file of the crates in part 5, with the file names and the "
        "crates it was found under.")
    for number, text, users, names in texts.values():
        heading = listing(sorted(set(names)), 6, ", ").splitlines()
        heading[0] = number + heading[0][len(number):]
        heading += listing(list(dict.fromkeys(users)), 6, ", ").splitlines()
        licence_texts += "\n" + text_block(heading, text)
    return crates, licence_texts


def std_part(toolchain):
    channel, sysroot = toolchain
    licences = {name: sysroot / f"share/doc/rust/licenses/{name}.txt" for name in ("Apache-2.0", "MIT")}
    for path in licences.values():
        if not path.is_file():
            die(f"{path} not found: the rustc component of the rustup toolchain ships it")
    out = para(
        f"The application is compiled with Rust {channel} (rust-toolchain.toml). The Rust "
        "standard library of that toolchain (std, core, alloc and the crates they are built "
        "from) is linked statically into gelabber-desktop and gelabber-desktop.exe.")
    out += "\n"
    out += para(
        "The Rust project states for it: licensed under Apache-2.0 or MIT, at the user's "
        "option, copyright The Rust Project Developers. The two texts below are the ones "
        "the toolchain ships for these licences, in its rustc component."
        + (
            " Its MIT text is the general form of that licence, with a placeholder where the "
            "copyright line stands."
            if "<copyright holders>" in read(licences["MIT"]) else ""
        ))
    out += "\n"
    out += para(
        "The standard library has third-party dependencies of its own. The toolchain "
        "describes them and their notices in share/doc/rust/COPYRIGHT-library.html, also in "
        "the rustc component. That document is not in the packages and is not reproduced "
        "here, and which of the crates it names are in the application was not established.")
    for name, path in licences.items():
        heading = [name, f"  Text      {path.relative_to(sysroot).as_posix()} of Rust {channel}"]
        out += "\n" + text_block(heading, read(path))
    return out


def scope_part():
    facts = build_facts()
    if facts != EXPECTED_FACTS:
        differing = [name for name in facts if facts[name] != EXPECTED_FACTS[name]]
        die(f"parts 1 and 2 describe another build configuration; no longer true: {differing}")
    pin = pins()
    out = "What is covered\n"
    out += para("The programs of the packages and what is linked statically into them:", 2)
    out += "\n"
    out += "    gelabber-desktop (Linux), gelabber-desktop.exe (Windows)\n"
    out += "        the application, written in Rust: parts 4 to 6\n"
    out += "    libgelabber_media.so (Linux), gelabber_media.dll (Windows)\n"
    out += "        the native media core, C++ behind a C interface: parts 2 and 3\n"
    out += "\n"
    out += para(
        "Gelabber's own code is not listed. This file is an inventory: what is included, "
        "which licence each component declares, and the licence texts the components "
        "ship. It does not assess whether the conditions of any licence are met.", 2)
    out += "\nWhat is not covered\n"
    out += item(
        "Libraries the programs load from the operating system when they run, for example "
        "glibc, GLib, GTK 3, WebKitGTK, libdbus, PipeWire and GStreamer on Linux, the "
        "system DLLs and the WebView2 Runtime on Windows. The libraries are not in the "
        "packages. Their header files are used when the programs are compiled, and inline "
        "functions and macros of those headers become part of the programs. The last entry "
        "of part 2 says what a search of the Linux media core found of them and prints the "
        "notices of those headers. Nothing else was collected for header files of system "
        "libraries or of the compilers.")
    out += item(
        "The web client. The application shows the pages of the Gelabber server it is "
        "connected to; they are not in the packages.")
    out += item(
        "Notices that stand only inside individual source files and not in a licence "
        "file, except where a part says that it collected them.")
    out += "\nHow this file was produced\n"
    out += para(
        "By desktop/packaging/third-party-notices.py in the Gelabber repository "
        f"({GELABBER_URL}); do not edit it by hand. The script's header says how to "
        "refresh it. Inputs:", 2)
    out += item(
        f"part 2: a checkout of libwebrtc at {pin['WEBRTC_COMMIT']} with a finished Linux "
        "build, read with libwebrtc's own licence generator and through ninja's record of "
        "that build, and a Linux build of the media core with its symbol table")
    out += item(
        "part 3: the sources that desktop/native/CMakeLists.txt fetched for a build of the "
        "media core, checked against the pins in desktop/native/libwebrtc.env and "
        "CMakeLists.txt")
    out += item(
        "parts 4 to 6: desktop/Cargo.lock through cargo tree and cargo metadata on Linux "
        "x86-64, the crate sources in the cargo registry, and the licence texts in the "
        "Rust toolchain that rust-toolchain.toml pins")
    out += para(
        "Nothing was built or run for Windows to produce it; what it says about the "
        "Windows package is derived from the build scripts and the lock file. What part 2 "
        "says about the contents of the Linux media core was found in a build made on the "
        "machine that generated it, not in the released library.", 2)
    out += "\nOpen points\n"
    out += item(
        "H.264 on Linux. libwebrtc is built with rtc_use_h264=true for Linux, so "
        "libgelabber_media.so contains OpenH264 (an H.264 encoder) and decoders of "
        "Chromium's FFmpeg build with the \"Chrome\" branding (H.264 and the others that "
        "this configuration enables); see the entries ffmpeg and openh264 in part 2. How "
        "the patent licensing of H.264, and of the other codecs in that FFmpeg "
        "configuration, applies to distributing this build has not been settled by the "
        "project (issue #165 in the Gelabber repository). This file does not answer that "
        "question.")
    out += item(
        "H.264 on Windows. The libwebrtc package for Windows is built with "
        "rtc_use_h264=true as well, but the media core is compiled without "
        "WEBRTC_USE_H264 there, so its own code refers to neither OpenH264 nor FFmpeg. "
        "That gelabber_media.dll holds no code of either was not checked on a Windows "
        "build when this file was written.")
    out += item(
        "Microsoft runtime in gelabber_media.dll. The DLL links Microsoft's C runtime and "
        "C++ standard library statically (CMAKE_MSVC_RUNTIME_LIBRARY MultiThreaded), and "
        "clang's compiler-rt builtins when the build finds them. Their versions and "
        "licence texts are not listed: they come from the Visual Studio and LLVM "
        "installation of the build machine.")
    out += item(
        "The Windows installer. It is made by Tauri's bundler with NSIS "
        "(desktop/app/tauri.bundle.windows.json) and embeds Microsoft's WebView2 Runtime "
        "bootstrapper (webviewInstallMode embedBootstrapper). The installer program, the "
        "NSIS plugins it uses and the bootstrapper are not listed: they are added when "
        "the installer is built.")
    out += item(
        "The Rust standard library's own dependencies are not listed, see part 4.")
    out += item(
        "Components whose sources carry no licence text are named in part 3 (JSON for "
        "Modern C++, the RNNoise model) and in part 5 (\"Crates without a licence text\", "
        "\"Below the root\"), with what their sources state instead.")
    return out


# --- the file -----------------------------------------------------------------

# A part that is kept names a digest of its inputs and one of its own text.
KEPT = (
    "[the script keeps this part while its inputs have the digest {}\n"
    " and its text has the digest {}]\n\n"
)


def text_digest(text):
    return sha256(text.encode())[:32]


def kept_parts(text):
    """The parts of an existing file that are marked as kept:
    number -> (inputs, digest of the text, text)."""
    found = {}
    starts = list(re.finditer(r"^#{78}\n(\d)\. [^\n]*\n#{78}\n\n", text, re.M))
    for index, start in enumerate(starts):
        end = starts[index + 1].start() if index + 1 < len(starts) else len(text)
        body = text[start.end():end]
        marker = re.match(re.escape(KEPT).replace(r"\{\}", r"(\w+)"), body)
        if marker:
            found[int(start[1])] = (marker[1], marker[2], body[marker.end():].strip("\n"))
    return found


def native_part(kept, number, inputs, source, make, flag, file_name):
    """A part on the native media core: made from its sources when they are
    given, else the one of the existing file, provided that its inputs are
    the current ones and that nobody changed its text."""
    if source:
        text = make(source).strip("\n")
        return KEPT.format(inputs, text_digest(text)) + text
    if kept.get(number, ("",))[0] != inputs:
        die(
            f"the inputs of part {number} changed or {file_name} does not have it yet: "
            f"run again with {flag} (see the header of this script)"
        )
    _, digest, text = kept[number]
    if text_digest(text) != digest:
        die(
            f"part {number} of {file_name} is not the text the script wrote (changed by hand "
            f"or damaged): restore it from git or run again with {flag}"
        )
    return KEPT.format(inputs, digest) + text


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    option = parser.add_argument
    option("--webrtc-src", help="libwebrtc checkout (the src directory) at the pinned commit")
    option("--webrtc-out", default="out/gelabber", help="its GN build directory (default: %(default)s)")
    option("--core-build", help=f"CMake build directory of the media core (_deps, {CORE_LIBRARY})")
    option("--output", type=Path, default=OUTPUT, help="default: %(default)s")
    option("--offline", action="store_true", help="pass --offline to cargo")
    option("--check", action="store_true", help="compare instead of writing; exit status 1 on a difference")
    args = parser.parse_args()
    if args.webrtc_src and not args.core_build:
        die(f"--webrtc-src needs --core-build: part 2 says what of libwebrtc is in {CORE_LIBRARY}")

    existing = read(args.output).replace("\r\n", "\n") if args.output.is_file() else ""
    kept = kept_parts(existing)
    toolchain = rust_toolchain()
    crates, licence_texts = rust_parts(args.offline)
    parts = [
        ("Scope, sources and open points", scope_part()),
        (
            "Native media core: libwebrtc and the libraries built with it",
            native_part(
                kept, 2, libwebrtc_fingerprint(), args.webrtc_src,
                lambda src: libwebrtc_part(src, args.webrtc_out, args.core_build),
                "--webrtc-src and --core-build", args.output.name,
            ),
        ),
        (
            "Native media core: other libraries",
            native_part(
                kept, 3, core_fingerprint(), args.core_build, core_part, "--core-build",
                args.output.name,
            ),
        ),
        ("Application: Rust standard library", std_part(toolchain)),
        ("Application: Rust crates", crates),
        ("Application: licence texts of the Rust crates", licence_texts),
    ]

    out = "Gelabber desktop: third-party notices\n" + "=" * 37 + "\n\n"
    out += para(
        "Third-party software built into the Gelabber desktop packages for Linux x64 and "
        "Windows x64, the licences it declares and the licence texts it ships.")
    out += "\n"
    out += "".join(f"  {number}. {title}\n" for number, (title, _) in enumerate(parts, 1))
    for number, (title, body) in enumerate(parts, 1):
        out += f"\n\n{'#' * WIDTH}\n{number}. {title}\n{'#' * WIDTH}\n\n" + body.strip("\n") + "\n"

    if args.check:
        if out != existing:
            sys.exit(f"{args.output} is out of date: run desktop/packaging/third-party-notices.py")
        print(f"{args.output} is up to date")
        return
    args.output.write_text(out, encoding="utf-8", newline="\n")
    print(f"wrote {args.output} ({len(out.encode()) // 1024} KiB)")


if __name__ == "__main__":
    main()
