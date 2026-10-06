#!/usr/bin/env python3
"""Local, slow acceptance checks; never use the operator's Compose stack.

Example: python3 docker/check-rust-cache.py --baseline-ref <main-sha> \
    --output /tmp/gelabber-cache-report --keep-images
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
import uuid


ROOT = Path(__file__).resolve().parents[1]
LOCAL_PACKAGES = {"gelabber-api", "gelabber-media", "gelabber-shared"}


def command(argv, **kwargs):
    return subprocess.run(argv, check=True, text=True, capture_output=True, **kwargs).stdout


def source_change(context, package, marker="cache-transition"):
    path = context / package / "src" / ("lib.rs" if package == "shared" else "main.rs")
    with path.open("a") as stream:
        stream.write(f"\n// Local Docker cache acceptance source mutation: {marker}.\n")


def version_change(context):
    manifest = context / "Cargo.toml"
    content = manifest.read_text()
    match = re.search(r'(?m)^version = "([^"]+)"$', content)
    if not match:
        raise RuntimeError("workspace version not found")
    old, new = match[1], "0.0.0-cache-acceptance"
    manifest.write_text(content[:match.start(1)] + new + content[match.end(1):])
    lock = context / "Cargo.lock"
    content = lock.read_text()
    count = 0
    blocks = content.split("[[package]]")
    for index, block in enumerate(blocks):
        if re.search(r'(?m)^name = "gelabber-(api|media|shared)"$', block):
            block, changed = re.subn(r'(?m)^version = "' + re.escape(old) + r'"$',
                                    f'version = "{new}"', block)
            count += changed
            blocks[index] = block
    if count != 3:
        raise RuntimeError(f"expected three local lockfile versions; found {count}")
    lock.write_text("[[package]]".join(blocks))


def feature_change(context, package):
    manifest = context / package / "Cargo.toml"
    content, changed = re.subn(r'(?m)^(serde = .*features = \[)"derive"(\].*)$',
                             r'\1"derive", "rc"\2', manifest.read_text())
    if changed != 1:
        raise RuntimeError("expected serde derive feature declaration")
    manifest.write_text(content)


def cache_evidence(log, package):
    stages = {}
    for match in re.finditer(r'(?m)^#(\d+) \[(dependencies|build) [^\]]+\] '
                            r'RUN cargo build --locked --release -p gelabber-' + package, log):
        vertex, stage = match[1], match[2]
        stages[stage] = {"vertex": vertex,
                         "cached": bool(re.search(r'(?m)^#' + vertex + r' CACHED$', log)),
                         "header": match[0]}
    compiled = sorted(set(re.findall(r'\bCompiling ([a-zA-Z0-9_-]+) v', log)))
    return {"stages": stages, "compiled_packages": compiled,
            "compiled_external_packages": sorted(set(compiled) - LOCAL_PACKAGES)}


class Runner:
    def __init__(self, args):
        self.args = args
        self.prefix = "gelabber-cache-" + uuid.uuid4().hex[:12]
        self.builders, self.images = [], []
        self.report = {"schema": 1, "run_id": self.prefix, "measurements": [],
                       "assertions": [], "performance": {}, "cleanup_errors": [],
                       "builder_details": {},
                       "runtime_readiness": "not_checked; run separately"}
        self.output = args.output.resolve()
        self.output.mkdir(parents=True, exist_ok=True)

    def save(self):
        (self.output / "report.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def check(self, name, passed, details):
        self.report["assertions"].append({"name": name, "passed": passed, "details": details})
        self.save()
        if not passed:
            raise RuntimeError(f"acceptance assertion failed: {name}: {details}")

    def builder(self, suffix):
        name = self.prefix + "-" + suffix
        self.builders.append(name)
        command(["docker", "buildx", "create", "--name", name, "--driver", "docker-container"])
        self.report["builder_details"][name] = command(["docker", "buildx", "inspect", "--bootstrap", name])
        self.save()
        return name

    def build(self, context, builder, package, mode, phase, round_number=0, export=False):
        label = f"{package}-{mode}-{round_number}-{phase}"
        tag = f"{self.prefix}/{package}:acceptance"
        if export and tag not in self.images:
            self.images.append(tag)
        argv = ["docker", "buildx", "build", "--builder", builder, "--progress=plain",
                "--file", str(context / package / "Dockerfile")]
        if export:
            argv += ["--load", "--tag", tag,
                     "--build-arg", "OCI_REVISION=" + self.report["head_sha"],
                     "--build-arg", "OCI_VERSION=v" + self.report["workspace_version"]]
        else:
            argv += ["--target", "build", "--output", "type=cacheonly"]
        argv.append(str(context))
        path = self.output / (label + ".log")
        print(f"Building {label}; log: {path}", flush=True)
        started = time.monotonic()
        with path.open("w") as stream:
            process = subprocess.Popen(argv, stdout=stream, stderr=subprocess.STDOUT, text=True)
            try:
                returncode = process.wait()
            except BaseException:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                raise
        record = {"package": package, "mode": mode, "phase": phase, "round": round_number,
                  "seconds": time.monotonic() - started, "returncode": returncode,
                  "image": tag if export else None, "builder": builder, "log": str(path), "command": argv}
        record.update(cache_evidence(path.read_text(), package))
        self.report["measurements"].append(record)
        self.save()
        if returncode:
            raise RuntimeError(f"build failed: {label}; inspect {path}")
        return record

    def transition(self, record, dependency_cached, build_cached, local_only=False):
        stages = record["stages"]
        name = f'{record["package"]}/{record["phase"]}'
        expected = {"dependencies": dependency_cached, "build": build_cached}
        self.check(name + "/cache", all(stage in stages and stages[stage]["cached"] == cached
                                        for stage, cached in expected.items()), {"expected": expected,
                                                                              "actual": stages})
        if local_only:
            self.check(name + "/compilation", not record["compiled_external_packages"]
                       and "gelabber-" + record["package"] in record["compiled_packages"],
                       record["compiled_packages"])

    def cache_checks(self, snapshot, work, package, builder=None):
        seeded = builder is not None
        builder = builder or self.builder(package + "-transitions")
        context = work / (package + "-transitions")
        shutil.copytree(snapshot, context)
        if not seeded:
            self.transition(self.build(context, builder, package, "cache", "cold"), False, False)
        self.transition(self.build(context, builder, package, "cache", "warm"), True, True)
        variants = [("own-source", lambda: source_change(context, package), True, False),
                    ("other-source", lambda: source_change(context, "media" if package == "api" else "api"), True, True),
                    ("shared-source", lambda: source_change(context, "shared"), True, False),
                    ("release-version", lambda: version_change(context), True, False),
                    ("external-feature", lambda: feature_change(context, package), False, False)]
        for phase, mutate, dependency_cached, build_cached in variants:
            shutil.rmtree(context)
            shutil.copytree(snapshot, context)
            mutate()
            record = self.build(context, builder, package, "cache", phase)
            self.transition(record, dependency_cached, build_cached,
                            local_only=dependency_cached and not build_cached)
        return builder

    def performance_checks(self, snapshot, work, package, baseline):
        samples = {mode: {phase: [] for phase in ("cold", "warm", "own-source")}
                   for mode in ("baseline", "optimized")}
        builders, contexts = {}, {}
        for mode in samples:
            builders[mode] = self.builder(f"{package}-{mode}")
            contexts[mode] = work / f"{package}-{mode}"
            shutil.copytree(snapshot, contexts[mode])
            if mode == "baseline":
                (contexts[mode] / package / "Dockerfile").write_text(baseline[package])
            record = self.build(contexts[mode], builders[mode], package, mode, "cold")
            samples[mode]["cold"].append(record["seconds"])
        for round_number in range(1, self.args.rounds + 1):
            # Alternate order to reduce systematic time-of-run bias.
            modes = ("baseline", "optimized") if round_number % 2 else ("optimized", "baseline")
            for mode in modes:
                builder, context = builders[mode], contexts[mode]
                for phase in ("own-source", "warm"):
                    if phase == "own-source":
                        source_change(context, package, f"performance-round-{round_number}")
                    record = self.build(context, builder, package, mode, phase, round_number)
                    samples[mode][phase].append(record["seconds"])
                    cached = phase == "warm"
                    self.check(f"{package}/{mode}/{round_number}/{phase}/build-cache",
                               record["stages"].get("build", {}).get("cached") == cached,
                               record["stages"])
                    if mode == "optimized":
                        self.transition(record, True, cached, local_only=phase == "own-source")
        medians = {mode: {phase: statistics.median(values) for phase, values in phases.items()}
                   for mode, phases in samples.items()}
        before, after = (medians[mode]["own-source"] for mode in ("baseline", "optimized"))
        improvement = 100 * (before - after) / before
        self.report["performance"][package] = {"samples_seconds": samples,
                                              "median_seconds": medians,
                                              "warm_source_improvement_percent": improvement}
        self.check(package + "/20-percent-warm-source-improvement", improvement >= 20,
                   {"measured_percent": improvement, "required_percent": 20})
        return builders["optimized"]

    def final_image(self, snapshot, work, package, builder):
        context = work / (package + "-final-image")
        shutil.copytree(snapshot, context)
        record = self.build(context, builder, package, "optimized", "final-image", export=True)
        record["image_id"] = command(["docker", "image", "inspect", "--format", "{{.Id}}", record["image"]]).strip()
        self.save()

    def cleanup(self):
        for builder in reversed(self.builders):
            result = subprocess.run(["docker", "buildx", "rm", "--force", builder],
                                    capture_output=True, text=True)
            if result.returncode:
                self.report["cleanup_errors"].append({"builder": builder, "error": result.stderr})
        if not self.args.keep_images:
            for image in self.images:
                result = subprocess.run(["docker", "image", "rm", "--force", image],
                                        capture_output=True, text=True)
                if result.returncode and "No such image" not in result.stderr:
                    self.report["cleanup_errors"].append({"image": image, "error": result.stderr})
        self.report["kept_images"] = self.images if self.args.keep_images else []
        self.save()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-ref", required=True, help="exact main commit before this cache change")
    parser.add_argument("--output", required=True, type=Path, help="directory for JSON and complete build logs")
    parser.add_argument("--mode", choices=("cache", "perf", "all"), default="all")
    parser.add_argument("--packages", nargs="+", choices=("api", "media"), default=["api", "media"])
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--keep-images", action="store_true", help="retain named images for separate runtime checks")
    args = parser.parse_args()
    if args.rounds < 3:
        parser.error("--rounds must be at least 3 for meaningful median performance evidence")
    runner = Runner(args)
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    try:
        baseline_sha = command(["git", "rev-parse", "--verify", "--end-of-options",
                                args.baseline_ref + "^{commit}"], cwd=ROOT).strip()
        baseline = {package: command(["git", "show", f"{baseline_sha}:{package}/Dockerfile"], cwd=ROOT)
                    for package in args.packages}
        runner.report.update({"baseline_sha": baseline_sha,
                              "head_sha": command(["git", "rev-parse", "HEAD"], cwd=ROOT).strip(),
                              "git_status": command(["git", "status", "--short"], cwd=ROOT),
                              "docker_version": command(["docker", "version"]),
                              "buildx_version": command(["docker", "buildx", "version"]),
                              "mode": args.mode, "rounds": args.rounds,
                              "timing_target": "build stage; no image export; one cold seed per implementation",
                              "workspace_version": re.search(r'(?m)^version = "([^"]+)"$',
                                                             (ROOT / "Cargo.toml").read_text())[1]})
        with tempfile.TemporaryDirectory(prefix=runner.prefix + "-") as temporary:
            work = Path(temporary)
            snapshot = work / "snapshot"
            files = command(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT)
            digests = {}
            for relative in set(files.split("\0")) - {""}:
                if (relative in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".dockerignore"}
                        or relative.startswith(("api/", "media/", "shared/", "docker/"))):
                    source, target = ROOT / relative, snapshot / relative
                    if source.is_file():
                        target.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(source, target)
                        digests[relative] = hashlib.sha256(target.read_bytes()).hexdigest()
            runner.report["context_sha256"] = digests
            runner.report["baseline_dockerfile_sha256"] = {
                package: hashlib.sha256(content.encode()).hexdigest() for package, content in baseline.items()}
            runner.save()
            for package in args.packages:
                builder = None
                if args.mode in ("perf", "all"):
                    builder = runner.performance_checks(snapshot, work, package, baseline)
                if args.mode in ("cache", "all"):
                    builder = runner.cache_checks(snapshot, work, package, builder)
                if args.keep_images:
                    runner.final_image(snapshot, work, package, builder)
        runner.report["passed"] = True
    except (Exception, KeyboardInterrupt, SystemExit) as error:
        runner.report.update({"passed": False, "error": str(error) or type(error).__name__})
        print(f"Acceptance failed: {error}", file=sys.stderr)
    finally:
        runner.cleanup()
    print(f"Report: {runner.output / 'report.json'}", flush=True)
    return 0 if runner.report.get("passed") and not runner.report["cleanup_errors"] else 1


if __name__ == "__main__":
    sys.exit(main())
