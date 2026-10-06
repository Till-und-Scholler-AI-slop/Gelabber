#!/usr/bin/env python3
"""Bootstrap verified native-build wheels without changing the host Python."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import urllib.request
import venv

LOCK = Path(__file__).with_name("mediasoup-build-tools.lock.json")
DEBIAN_LOCK = Path(__file__).with_name("mediasoup-debian-build-tools.lock.tsv")


def debian_packages(architecture):
    target = {"x86_64": "amd64", "aarch64": "arm64"}.get(architecture)
    if target is None:
        raise ValueError("unsupported native-build architecture: " + architecture)
    records = [line.split("\t") for line in DEBIAN_LOCK.read_text().splitlines()
               if line and not line.startswith("#")]
    if any(len(row) != 6 for row in records):
        raise ValueError("invalid native Debian archive lock")
    selected = [row for row in records if row[0] == target]
    if not selected or len({row[1] for row in selected}) != len(selected):
        raise ValueError("empty or duplicate native Debian archive lock")
    return [f"{row[1]}={row[2]}" for row in selected]


def native_launcher_source(python):
    # The SDK computes Meson jobs from inherited CPU affinity, independently of
    # cargo's job count. Apply the same bound before any upstream Python starts.
    return f'''#!/usr/bin/env python3
import os
import sys
jobs = int(os.environ.get("CARGO_BUILD_JOBS", "2"))
if not 1 <= jobs <= 2:
    raise SystemExit("native build supports one or two concurrent jobs")
if not hasattr(os, "sched_getaffinity") or not hasattr(os, "sched_setaffinity"):
    raise SystemExit("native build needs Linux CPU affinity support")
allowed = sorted(os.sched_getaffinity(0))
os.sched_setaffinity(0, set(allowed[:jobs]))
python = {str(python)!r}
os.execv(python, [python, *sys.argv[1:]])
'''


def selected_wheels(lock, architecture):
    if architecture not in {"x86_64", "aarch64"}:
        raise ValueError(f"unsupported native-build architecture: {architecture}")
    return [(tool, tool["wheels"].get("universal") or tool["wheels"][architecture])
            for tool in lock["tools"]]


def bootstrap(destination):
    lock_bytes = LOCK.read_bytes()
    lock = json.loads(lock_bytes)
    wheels = selected_wheels(lock, platform.machine())
    digest = hashlib.sha256(lock_bytes).hexdigest()
    destination = destination.resolve()
    if destination.exists():
        raise ValueError("native-build destination must be new; use the existing verified environment or a fresh path")
    destination.mkdir(parents=True)
    wheelhouse = destination / "wheelhouse"
    wheelhouse.mkdir()
    for tool, wheel in wheels:
        filename = wheel["filename"]
        if Path(filename).name != filename or not filename.endswith(".whl"):
            raise ValueError("invalid locked wheel filename")
        url = wheel["url"]
        if not url.startswith("https://files.pythonhosted.org/"):
            raise ValueError("unexpected wheel origin")
        data = urllib.request.urlopen(url, timeout=60).read()
        if hashlib.sha256(data).hexdigest() != wheel["sha256"]:
            raise ValueError(f"native-build wheel checksum mismatch: {filename}")
        (wheelhouse / filename).write_bytes(data)
    requirements = destination / "requirements.txt"
    requirements.write_text("".join(f'{tool["name"]}=={tool["version"]} --hash=sha256:{wheel["sha256"]}\n'
                                    for tool, wheel in wheels))
    constraints = destination / "constraints.txt"
    constraints.write_text("".join(f'{tool["name"]}=={tool["version"]}\n' for tool, _ in wheels))
    environment = destination / "venv"
    venv.EnvBuilder(with_pip=True).create(environment)
    python = environment / "bin" / "python"
    subprocess.run([str(python), "-m", "pip", "install", "--no-index", "--no-deps", "--force-reinstall",
                    "--find-links", str(wheelhouse), "--require-hashes", "-r", str(requirements)], check=True)
    observed = json.loads(subprocess.check_output([str(python), "-c",
        "import importlib.metadata,json; print(json.dumps({name:importlib.metadata.version(name) "
        "for name in ['invoke','pip','setuptools','meson','ninja']}))"], text=True))
    expected = {tool["name"]: tool["version"] for tool, _ in wheels}
    if observed != expected:
        raise ValueError("installed native-build tools differ from lock")
    launcher = destination / "native-python"
    launcher.write_text(native_launcher_source(python))
    launcher.chmod(0o755)
    record = {"lock_sha256": digest, "architecture": platform.machine(), "tools": observed,
              "python": str(python), "native_python": str(launcher), "native_build_jobs": 2,
              "python_version": subprocess.check_output([str(python), "--version"], text=True).strip()}
    (destination / "verified.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path)
    parser.add_argument("--debian-packages", action="store_true")
    args = parser.parse_args()
    if args.debian_packages:
        print(" ".join(debian_packages(platform.machine())))
    elif args.destination:
        bootstrap(args.destination)
    else:
        parser.error("choose --destination or --debian-packages")


if __name__ == "__main__":
    main()
