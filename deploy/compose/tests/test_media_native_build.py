"""Native bootstrap integrity and image-smoke failure contracts; no Docker starts."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]


def load_script(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "docker" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


build = load_script("mediasoup-build-tools")
smoke = load_script("check-media-image")


class NativeBootstrap(unittest.TestCase):
    def test_canonical_debian_lock_selects_existing_compiler_and_python_payloads(self):
        for architecture, triplet in (("x86_64", "x86-64-linux-gnu"), ("aarch64", "aarch64-linux-gnu")):
            packages = dict(value.split("=", 1) for value in build.debian_packages(architecture))
            self.assertEqual(len(packages), 16)
            self.assertEqual(packages["gcc"], "4:14.2.0-1")
            self.assertEqual(packages["gcc-14-" + triplet], packages["gcc-14"])
            self.assertEqual(packages["g++-14-" + triplet], packages["g++-14"])
            for package in ("python3.13", "python3.13-minimal", "python3.13-venv",
                            "libpython3.13-minimal", "libpython3.13-stdlib"):
                self.assertEqual(packages[package], "3.13.5-2+deb13u5")
        with self.assertRaisesRegex(ValueError, "unsupported"):
            build.debian_packages("armv7l")

    def test_checksum_mismatch_cannot_create_or_run_an_environment(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "new"
            with patch.object(build.urllib.request, "urlopen", return_value=io.BytesIO(b"tampered wheel")), \
                    patch.object(build.venv, "EnvBuilder") as environment, \
                    patch.object(build.subprocess, "run") as command:
                with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                    build.bootstrap(destination)
                environment.assert_not_called()
                command.assert_not_called()
                self.assertFalse((destination / "verified.json").exists())
                self.assertEqual(list((destination / "wheelhouse").iterdir()), [])

    def test_existing_environment_is_never_rewritten(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "marker"
            marker.write_text("existing environment")
            with patch.object(build.urllib.request, "urlopen") as download:
                with self.assertRaisesRegex(ValueError, "must be new"):
                    build.bootstrap(Path(directory))
                download.assert_not_called()
                self.assertEqual(marker.read_text(), "existing environment")

    def test_only_supported_architectures_get_arch_specific_ninja(self):
        lock = json.loads(build.LOCK.read_text())
        for architecture in ("x86_64", "aarch64"):
            wheels = dict((tool["name"], wheel) for tool, wheel in build.selected_wheels(lock, architecture))
            self.assertIn(architecture, wheels["ninja"]["filename"])
            self.assertNotIn(architecture, wheels["meson"]["filename"])
        with self.assertRaisesRegex(ValueError, "unsupported"):
            build.selected_wheels(lock, "armv7l")

    def test_native_python_restricts_inherited_affinity_and_refuses_more_than_two_jobs(self):
        with tempfile.TemporaryDirectory() as directory:
            launcher = Path(directory) / "native-python"
            launcher.write_text(build.native_launcher_source(sys.executable))
            launcher.chmod(0o755)
            for jobs in ("1", "2"):
                result = subprocess.run([str(launcher), "-c", "import os;print(len(os.sched_getaffinity(0)))"],
                                        env={**os.environ, "CARGO_BUILD_JOBS": jobs}, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(int(result.stdout), min(int(jobs), len(os.sched_getaffinity(0))))
            result = subprocess.run([str(launcher), "-c", "raise SystemExit('must not execute')"],
                                    env={**os.environ, "CARGO_BUILD_JOBS": "3"}, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("one or two concurrent jobs", result.stderr)


class DebianArchiveIntegrity(unittest.TestCase):
    """Execute the real shell/hash gate with a controlled dpkg metadata reader."""
    def verify(self, *, checksum=None, metadata=None, duplicate=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archives = root / "archives"
            archives.mkdir()
            archive = archives / "build-essential.deb"
            archive.write_bytes(b"native tool archive fixture")
            expected = checksum or hashlib.sha256(archive.read_bytes()).hexdigest()
            row = "\t".join(["amd64", "build-essential", "12.12", "amd64", expected,
                              "pool/main/b/build-essential/build-essential_12.12_amd64.deb"]) + "\n"
            lock = root / "lock.tsv"
            lock.write_text(row * (2 if duplicate else 1))
            commands = root / "commands"
            commands.mkdir()
            marker = root / "dpkg-calls"
            reader = commands / "dpkg-deb"
            reader.write_text("#!/usr/bin/env python3\nimport json,os,sys\n"
                              "with open(os.environ['APT_TEST_MARKER'],'a') as output: output.write(sys.argv[-1]+'\\n')\n"
                              "print(json.loads(os.environ['APT_TEST_METADATA'])[sys.argv[-1]])\n")
            reader.chmod(0o755)
            result = subprocess.run(["sh", str(ROOT / "docker/install-mediasoup-debian-build-tools.sh"),
                                     "--verify-only", str(lock), str(archives), "amd64"],
                                    env={**os.environ, "PATH": str(commands) + os.pathsep + os.environ["PATH"],
                                         "APT_TEST_MARKER": str(marker), "APT_TEST_METADATA": json.dumps(metadata or {
                                             "Package": "build-essential", "Version": "12.12", "Architecture": "amd64"})},
                                    capture_output=True, text=True, timeout=10)
            return result, marker.read_text() if marker.exists() else ""

    def test_changed_archive_bytes_are_rejected_before_dpkg_metadata_is_used(self):
        result, calls = self.verify(checksum="0" * 64)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertEqual(calls, "")

    def test_wrong_package_version_or_architecture_is_rejected_after_hash_verification(self):
        for field, wrong in (("Package", "foreign"), ("Version", "12.13"), ("Architecture", "arm64")):
            with self.subTest(field=field):
                metadata = {"Package": "build-essential", "Version": "12.12", "Architecture": "amd64", field: wrong}
                result, calls = self.verify(metadata=metadata)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(field.lower() + " mismatch", result.stderr)
                self.assertIn(field, calls)

    def test_valid_archive_gate_succeeds_and_duplicate_lock_cannot_run_metadata(self):
        result, calls = self.verify()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls.splitlines(), ["Package", "Version", "Architecture"])
        result, calls = self.verify(duplicate=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("invalid or empty", result.stderr)
        self.assertEqual(calls, "")


class NativeCiBoundary(unittest.TestCase):
    """Execute the wrapper with a recording Docker boundary, never a daemon."""
    def run_wrapper(self, *, image="sha256:" + "a" * 64, exit_code=0):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "checkout with spaces"
            workspace.mkdir()
            (workspace / "Cargo.toml").write_text("[workspace]\n")
            cache = root / "cargo cache"
            commands = root / "commands"
            commands.mkdir()
            calls = root / "docker-calls.jsonl"
            docker = commands / "docker"
            docker.write_text("#!/usr/bin/env python3\nimport json,os,sys\n"
                              "with open(os.environ['CI_TEST_CALLS'],'a') as output: output.write(json.dumps(sys.argv[1:])+'\\n')\n"
                              "if sys.argv[1:3]==['image','inspect']: print(os.environ['CI_TEST_IMAGE'])\n"
                              "else: sys.exit(int(os.environ['CI_TEST_EXIT']))\n")
            docker.chmod(0o755)
            result = subprocess.run(["sh", str(ROOT / "docker/run-native-ci.sh"), "test", "--locked",
                                     "--workspace", "--", "--exact", "literal;argument"],
                                    env={**os.environ, "PATH": str(commands) + os.pathsep + os.environ["PATH"],
                                         "GITHUB_WORKSPACE": str(workspace), "CARGO_HOME": str(cache),
                                         "PYTHON": "/foreign/host/python", "CI_TEST_CALLS": str(calls),
                                         "CI_TEST_IMAGE": image, "CI_TEST_EXIT": str(exit_code)},
                                    capture_output=True, text=True, timeout=10)
            return result, [json.loads(line) for line in calls.read_text().splitlines()], workspace, cache

    def test_pinned_image_uid_cache_and_target_keep_host_tools_out_of_pr_gate(self):
        result, calls, workspace, cache = self.run_wrapper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[0], ["image", "inspect", "--format", "{{.Id}}", "gelabber-native-ci-toolchain:local"])
        run = calls[1]
        self.assertEqual(run[run.index("--network") + 1], "host")
        self.assertEqual(run[run.index("--user") + 1], f"{os.getuid()}:{os.getgid()}")
        self.assertEqual(run[run.index("--workdir") + 1], str(workspace))
        self.assertIn(str(workspace) + ":" + str(workspace), run)
        self.assertIn(str(cache) + ":/cargo", run)
        self.assertIn("CARGO_TARGET_DIR=" + str(workspace / "target/native-debian"), run)
        self.assertIn("RUSTUP_HOME=/usr/local/rustup", run)
        self.assertIn("CARGO_HOME=/cargo", run)
        self.assertNotIn("PYTHON", run)
        self.assertNotIn("/foreign/host/python", run)
        self.assertEqual(run[-8:], ["sha256:" + "a" * 64, "cargo", "test", "--locked", "--workspace", "--", "--exact", "literal;argument"])

    def test_cargo_failure_propagates_without_retry(self):
        result, calls, _, _ = self.run_wrapper(exit_code=42)
        self.assertEqual(result.returncode, 42)
        self.assertEqual(sum(call[0] == "run" for call in calls), 1)

    def test_malformed_or_mutable_image_identity_cannot_launch_cargo(self):
        for image in ("mutable:latest", "sha256:" + "a" * 63, "sha256:" + "g" * 64,
                      "sha256:" + "a" * 64 + "\nforeign"):
            with self.subTest(image=image):
                result, calls, _, _ = self.run_wrapper(image=image)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(len(calls), 1)


class FakeRuntime:
    """Stateful Docker/HTTP stand-in to test cleanup and fail-closed gates."""
    def __init__(self, media_ready=True, exit_code=0, term_stalls=False):
        self.revision = "a" * 40
        self.media_ready = media_ready
        self.exit_code = exit_code
        self.term_stalls = term_stalls
        self.containers = {}
        self.network = None
        self.calls = []
        self.clock = 0

    def inspect(self, kind, identity):
        if kind == "image":
            return {"Id": "media-image" if identity == "media:local" else "redis-image",
                    "Config": {"Labels": {"org.opencontainers.image.revision": self.revision}}}
        if kind == "network":
            return copy.deepcopy(self.network)
        return copy.deepcopy(next((record for record in self.containers.values()
                                   if identity in (record["Id"], record["Name"].lstrip("/"))), None))

    def docker(self, *arguments, **kwargs):
        self.calls.append(arguments)
        if arguments[:2] == ("network", "create"):
            label = arguments[arguments.index("--label") + 1].split("=", 1)[1]
            self.network = {"Id": "network-id", "Name": arguments[-1],
                            "Labels": {smoke.OWNER_LABEL: label}, "Containers": {}}
            return "network-id"
        if arguments[0] == "run":
            name = arguments[arguments.index("--name") + 1]
            label = arguments[arguments.index("--label") + 1].split("=", 1)[1]
            self.containers[name] = {"Id": name + "-id", "Name": "/" + name, "Image": arguments[-1],
                                    "Config": {"Labels": {smoke.OWNER_LABEL: label}},
                                    "State": {"Running": True, "Status": "running", "ExitCode": 0},
                                    "NetworkSettings": {"Ports": {"8081/tcp": [
                                        {"HostIp": "127.0.0.1", "HostPort": "39001"}]}}}
            return name + "-id"
        if arguments[0] in {"kill", "stop", "rm"}:
            name, record = next((name, record) for name, record in self.containers.items()
                                if record["Id"] == arguments[-1])
            if arguments[0] == "rm":
                del self.containers[name]
            elif not (arguments[0] == "kill" and self.term_stalls):
                record["State"] = {"Running": False, "Status": "exited",
                                   "ExitCode": self.exit_code if name.endswith("-media") else 0}
            return record["Id"]
        if arguments[:2] == ("network", "rm"):
            self.network = None
            return "network-id"
        raise AssertionError(f"unexpected fake Docker command {arguments}")

    def monotonic(self):
        return self.clock

    def sleep(self, seconds):
        self.clock += seconds

    def open(self, address, timeout):
        response = io.BytesIO(json.dumps({"status": "ready", "checks": {
            "redis": {"status": "ok"}, **({"media": {"status": "ok"}} if self.media_ready else {})}}).encode())
        response.status = 200
        return response

    def run(self):
        with patch.object(smoke, "docker", side_effect=self.docker), \
                patch.object(smoke, "inspect", side_effect=self.inspect), \
                patch.object(smoke.time, "monotonic", side_effect=self.monotonic), \
                patch.object(smoke.time, "sleep", side_effect=self.sleep), \
                patch.object(smoke.urllib.request, "build_opener") as opener:
            opener.return_value.open.side_effect = self.open
            return smoke.smoke("media:local", self.revision)


class ImageSmoke(unittest.TestCase):
    def test_only_exact_requested_resource_not_found_proves_absence(self):
        for kind in ("container", "image", "network"):
            messages = [f"Error response from daemon: No such {kind}: owned-id\n"]
            if kind == "network":
                messages.append("Error response from daemon: network owned-id not found\n")
            for message in messages:
                with self.subTest(kind=kind, message=message), \
                        patch.object(smoke, "docker", side_effect=subprocess.CalledProcessError(
                            1, ["docker", kind, "inspect", "owned-id"], stderr=message)):
                    self.assertIsNone(smoke.inspect(kind, "owned-id"))
        for message in ("Cannot connect to the Docker daemon", "No such socket",
                        "Error response from daemon: network foreign-id not found",
                        "Error response from daemon: No such container: foreign-id"):
            with self.subTest(message=message), \
                    patch.object(smoke, "docker", side_effect=subprocess.CalledProcessError(
                        1, ["docker", "network", "inspect", "owned-id"], stderr=message)):
                with self.assertRaises(subprocess.CalledProcessError):
                    smoke.inspect("network", "owned-id")

    def test_real_ready_contract_and_normal_exit_clean_every_owned_resource(self):
        runtime = FakeRuntime()
        report = runtime.run()
        self.assertEqual(report["status"], "PASS")
        self.assertEqual(report["ready_checks"], {"redis": "ok", "media": "ok"})
        self.assertTrue(report["normal_shutdown"])
        self.assertFalse(report["forced_cleanup"])
        self.assertEqual(set(report["resources"]), {"network", "redis", "media"})
        self.assertEqual(set(report["cleanup"]["verified_absent"]),
                         {resource["id"] for resource in report["resources"].values()})
        self.assertEqual(runtime.containers, {})
        self.assertIsNone(runtime.network)
        media_run = next(call for call in runtime.calls if call[0] == "run" and call[-1] == "media-image")
        self.assertEqual(media_run[media_run.index("--publish") + 1], "127.0.0.1::8081")

    def test_redis_only_readiness_fails_and_cleans_running_media(self):
        runtime = FakeRuntime(media_ready=False)
        report = runtime.run()
        self.assertEqual(report["status"], "FAIL")
        self.assertFalse(report["ready"])
        self.assertTrue(report["forced_cleanup"])
        self.assertTrue(report["cleanup"]["complete"])
        self.assertEqual(runtime.containers, {})
        self.assertGreaterEqual(runtime.clock, 60)

    def test_abnormal_or_stalled_media_exit_cannot_pass(self):
        for arguments in ({"exit_code": 137}, {"term_stalls": True}):
            with self.subTest(arguments=arguments):
                runtime = FakeRuntime(**arguments)
                report = runtime.run()
                self.assertEqual(report["status"], "FAIL")
                self.assertFalse(report["normal_shutdown"])
                self.assertTrue(report["cleanup"]["complete"])
                self.assertEqual(runtime.containers, {})

    def test_foreign_container_and_non_loopback_binding_are_rejected(self):
        record = {"Name": "/owned", "Image": "expected", "Id": "id",
                  "Config": {"Labels": {smoke.OWNER_LABEL: "foreign"}}}
        with self.assertRaisesRegex(ValueError, "ownership"):
            smoke.owned_container(record, name="owned", image="expected", token="owned", identity="id")
        for bindings in ({"8081/tcp": [{"HostIp": "0.0.0.0", "HostPort": "39001"}]},
                         {"8081/tcp": [{"HostIp": "127.0.0.1", "HostPort": "39001"}],
                          "10000/udp": [{"HostIp": "127.0.0.1", "HostPort": "10000"}]}):
            with self.assertRaisesRegex(ValueError, "loopback"):
                smoke.http_address({"NetworkSettings": {"Ports": bindings}})


if __name__ == "__main__":
    unittest.main()
