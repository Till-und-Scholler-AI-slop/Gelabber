#!/usr/bin/env python3
"""Writes desktop/app/package/THIRD-PARTY-NOTICES.txt: the third-party
software built into the desktop packages, with the licence texts it ships.

  desktop/packaging/third-party-notices.py
      Rust parts afresh from desktop/Cargo.lock (cargo tree, cargo metadata
      and the crate sources in the cargo registry). The two parts on the
      native media core are kept from the existing file as long as their
      pins have not changed.
  desktop/packaging/third-party-notices.py \\
      --webrtc-src <work-dir>/src --core-build <cmake build dir of the core>
      Everything afresh. <work-dir> is the one of build-libwebrtc-linux.sh
      (a finished Linux build, out/gelabber); the CMake build directory is
      <target>/<profile>/build/gelabber-media-core-*/out/build.
  desktop/packaging/third-party-notices.py --check
      Exit status 1 when the committed file is not what the first form
      would write.

Needs python3, git and cargo; no network beyond what cargo needs to read
the crates of the lock file. Nothing is written outside the output file.
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
WIDTH = 78

# Raise when the wording or layout of a kept part changes, so that an old
# copy of it is not taken over.
LIBWEBRTC_PART = 1
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

# Licence files libwebrtc's generator is given beyond the one that a
# library's README.chromium names (only for libraries its table lacks).
EXTRA_LICENCE_FILES = {"ffmpeg": ["third_party/ffmpeg/COPYING.LGPLv2.1"]}

LICENCE_NAME = re.compile(r"(licen[sc]e|copying|copyright|notice|unlicense|patents)", re.I)
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
                    table[name] = [
                        str((readme.parent / f.strip()).relative_to(src))
                        for f in fields["License File"].split(",")
                    ] + EXTRA_LICENCE_FILES.get(name, [])
                extra = [
                    f"{n} also {', '.join(EXTRA_LICENCE_FILES[n])}" for n in names if n in EXTRA_LICENCE_FILES
                ]
                notes.append(
                    f"The generator's table has no entry for {' and '.join(names)}: it stops with "
                    f"\"{error}\". For these libraries it was given the licence file that the "
                    "library's README.chromium names in the field \"License File\""
                    + (f" (for {'; '.join(extra)})" if extra else "") + "."
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


def ffmpeg_configuration(src, gn_args):
    branding = next(a.split("=", 1)[1].strip('"') for a in gn_args if a.startswith("ffmpeg_branding="))
    config = Path("third_party/ffmpeg/chromium/config") / branding / "linux/x64"
    header = read(src / config / "config.h")
    licence = re.search(r'#define FFMPEG_LICENSE "(.*)"', header)[1]
    switches = {
        name: re.search(rf"#define CONFIG_{name} (\d)", header)[1] for name in ("GPL", "NONFREE", "VERSION3")
    }
    enabled = {}
    components = read(src / config / "config_components.h")
    for name, kind in re.findall(r"#define CONFIG_(\w+)_(DECODER|PARSER|DEMUXER|ENCODER|MUXER) 1", components):
        enabled.setdefault(kind.lower() + "s", []).append(name.lower())
    out = para(
        f"Licence of this build as its configuration states it ({config}/config.h): "
        f"FFMPEG_LICENSE \"{licence}\", CONFIG_GPL {switches['GPL']}, CONFIG_NONFREE "
        f"{switches['NONFREE']}, CONFIG_VERSION3 {switches['VERSION3']}.")
    out += para(
        "Components that configuration enables, which are compiled into the libwebrtc "
        "package (the linker decides which of them reach the media core):")
    for kind in sorted(enabled):
        out += f"  {kind}:\n" + listing(sorted(enabled[kind]), 4)
    return out


def libwebrtc_part(src, out_name):
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
        "This is a list of dependencies. It can name more than the binary holds: the linker "
        "drops code that nothing references, and tools of the build are dependencies too "
        "(nasm is an assembler). \"From\" is the git repository and commit the licence file "
        "was checked out from; apart from the change named above, none of these checkouts "
        "differed from its commit. The other fields quote the README.chromium that "
        "libwebrtc's tree keeps for the library.", 2)
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
        "lacks was not established.", 2)
    out += "\nLibraries\n"
    out += listing(names, 2)

    for name, files in libraries:
        folder = (src / files[0]).parent
        top = Path(git(folder, "rev-parse", "--show-toplevel"))
        if top != src and git(top, "status", "--porcelain", "--untracked-files=no"):
            die(f"{top} has local changes")
        readme, fields = chromium_readme(src, files[0])
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
        if name != "webrtc":
            for label, field in (("Name", "Name"), ("Version", "Version"), ("Revision", "Revision")):
                if fields.get(field, "N/A") not in ("N/A", "DEPS"):
                    heading.append(f"  {label:<9} {fields[field]}")
        if fields.get("License"):
            heading.append(f"  Declared  {fields['License']}")
        heading.append(f"  Text      {', '.join(files)}")
        body = ""
        if name == "ffmpeg":
            body += para(
                "FFmpeg is linked statically, as part of libwebrtc, into libgelabber_media.so "
                "(Linux). It is Chromium's copy of FFmpeg, unchanged. The source code is the "
                "repository and commit named above; the commands that build it into the media "
                "core are desktop/native/scripts/build-libwebrtc-linux.sh "
                f"and desktop/native/CMakeLists.txt in the Gelabber repository ({GELABBER_URL}) "
                "at the tag of the release.")
            body += ffmpeg_configuration(src, gn_args) + "\n"
        if name == "openh264":
            body += para(
                "OpenH264 is linked statically, as part of libwebrtc, into libgelabber_media.so "
                "(Linux), built from the source named above. The README.chromium of libwebrtc's "
                "tree notes for it that licences other than the BSD licence of the source apply "
                "to builds (MPEG LA patents) and refers to www.openh264.org.") + "\n"
        for file in files:
            if len(files) > 1:
                body += f"[{file}]\n\n"
            body += clean(read(src / file)) + "\n"
        out += "\n" + text_block(heading, body)
    return out


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
        for node in metadata["resolve"]["nodes"]:
            features.setdefault(node["id"], set()).update(node["features"])
        in_binary = cargo_tree(target, "normal,no-proc-macro", offline)
        for key in in_binary:
            linked.setdefault(key, []).append(system)
        build_time |= cargo_tree(target, "normal,build", offline)

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

    crates += "\nBuild time only\n"
    crates += para(
        "Procedural macros, build scripts and what those depend on. They run on the "
        "machine that builds the application; their own code is not linked into it, the "
        "code they generate is. As reported by cargo for the same two targets (-e "
        "normal,build, without the crates listed above). No texts are reproduced for "
        "them.", 2)
    crates += "\n"
    for key in third_party(build_time - set(linked)):
        package = packages[key]
        crates += f"  {key[0]} {key[1]} | {package['license'] or 'see ' + str(package['license_file'])}\n"

    licence_texts = para(
        "Each distinct licence file of the crates in part 5, with the file names and the "
        "crates it was found under.")
    for number, text, users, names in texts.values():
        heading = listing(sorted(set(names)), 6, ", ").splitlines()
        heading[0] = number + heading[0][len(number):]
        heading += listing(list(dict.fromkeys(users)), 6, ", ").splitlines()
        licence_texts += "\n" + text_block(heading, text)
    return crates, licence_texts


def std_part():
    channel = re.search(r'channel\s*=\s*"([^"]+)"', read(REPO / "rust-toolchain.toml"))[1]
    out = para(
        f"The application is compiled with Rust {channel} (rust-toolchain.toml). The Rust "
        "standard library of that toolchain (std, core, alloc and the crates they are built "
        "from) is linked statically into gelabber-desktop and gelabber-desktop.exe.")
    out += "\n"
    out += para(
        "The Rust project states for it: licensed under Apache-2.0 or MIT, at the user's "
        "option, copyright The Rust Project Developers. The standard library has "
        "third-party dependencies of its own. The Rust project documents them and their "
        "notices in the file share/doc/rust/COPYRIGHT-library.html of the installed "
        "toolchain. That document is not reproduced here.")
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
        "system DLLs and the WebView2 Runtime on Windows. They are not in the packages.")
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
        "build, read with libwebrtc's own licence generator")
    out += item(
        "part 3: the sources that desktop/native/CMakeLists.txt fetched for a build of the "
        "media core, checked against the pins in desktop/native/libwebrtc.env and "
        "CMakeLists.txt")
    out += item(
        "parts 4 to 6: desktop/Cargo.lock through cargo tree and cargo metadata, and the "
        "crate sources in the cargo registry")
    out += para(
        "Nothing was built or run for Windows to produce it; what it says about the "
        "Windows package is derived from the build scripts and the lock file.", 2)
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

KEPT = "[inputs {}: the script keeps this part while they are unchanged]\n\n"


def kept_parts(text):
    """The parts of an existing file that name their inputs: number -> (inputs, body)."""
    found = {}
    starts = list(re.finditer(r"^#{78}\n(\d)\. [^\n]*\n#{78}\n\n", text, re.M))
    for index, start in enumerate(starts):
        end = starts[index + 1].start() if index + 1 < len(starts) else len(text)
        body = text[start.end():end]
        marker = re.match(re.escape(KEPT).replace(r"\{\}", r"(\w+)"), body)
        if marker:
            found[int(start[1])] = (marker[1], body[marker.end():])
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    option = parser.add_argument
    option("--webrtc-src", help="libwebrtc checkout (the src directory) at the pinned commit")
    option("--webrtc-out", default="out/gelabber", help="its GN build directory (default: %(default)s)")
    option("--core-build", help="CMake build directory of the media core (contains _deps)")
    option("--output", type=Path, default=OUTPUT, help="default: %(default)s")
    option("--offline", action="store_true", help="pass --offline to cargo")
    option("--check", action="store_true", help="compare instead of writing; exit status 1 on a difference")
    args = parser.parse_args()

    existing = read(args.output).replace("\r\n", "\n") if args.output.is_file() else ""
    kept = kept_parts(existing)

    def native_part(number, inputs, source, make, flag):
        if source:
            return KEPT.format(inputs) + make(source)
        if kept.get(number, ("",))[0] != inputs:
            die(
                f"the inputs of part {number} changed or {args.output.name} does not have it yet: "
                f"run again with {flag} (see the header of this script)"
            )
        return KEPT.format(inputs) + kept[number][1]

    crates, licence_texts = rust_parts(args.offline)
    parts = [
        ("Scope, sources and open points", scope_part()),
        (
            "Native media core: libwebrtc and the libraries built with it",
            native_part(
                2, libwebrtc_fingerprint(), args.webrtc_src,
                lambda src: libwebrtc_part(src, args.webrtc_out), "--webrtc-src",
            ),
        ),
        (
            "Native media core: other libraries",
            native_part(3, core_fingerprint(), args.core_build, core_part, "--core-build"),
        ),
        ("Application: Rust standard library", std_part()),
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
