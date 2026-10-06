#!/usr/bin/env python3
"""Exercise actual media image readiness and SIGTERM, using owned Docker resources."""
import argparse
import json
import re
import subprocess
import time
import urllib.error
import urllib.request
import uuid

OWNER_LABEL = "io.gelabber.media-image-smoke"
REDIS_IMAGE = "redis:8.10.1-alpine"


def docker(*arguments, timeout=20):
    result = subprocess.run(["docker", *arguments], capture_output=True, text=True,
                            timeout=timeout, check=True)
    return result.stdout.strip()


def inspect(kind, identity):
    try:
        return json.loads(docker(kind, "inspect", identity))[0]
    except subprocess.CalledProcessError as error:
        # Only an explicit Docker not-found result proves absence. Daemon errors
        # must not be converted into a successful cleanup report.
        absence = f"Error response from daemon: No such {kind}: {identity}"
        network_absence = f"Error response from daemon: network {identity} not found"
        if error.stderr.strip() == absence or (kind == "network" and error.stderr.strip() == network_absence):
            return None
        raise


def owned_container(record, *, name, image, token, identity=None):
    if (record.get("Name") != "/" + name
            or record.get("Image") != image
            or record.get("Config", {}).get("Labels", {}).get(OWNER_LABEL) != token
            or (identity is not None and record.get("Id") != identity)):
        raise ValueError("container ownership or image changed")
    return record["Id"]


def owned_network(record, *, name, token, identity=None):
    if (record.get("Name") != name or record.get("Labels", {}).get(OWNER_LABEL) != token
            or (identity is not None and record.get("Id") != identity)):
        raise ValueError("network ownership changed")
    return record["Id"]


def ready_payload(payload):
    checks = payload.get("checks", {}) if isinstance(payload, dict) else {}
    return (isinstance(payload, dict) and payload.get("status") == "ready"
            and all(isinstance(checks.get(name), dict)
                    and checks[name].get("status") == "ok" for name in ("redis", "media")))


def http_address(record):
    ports = record.get("NetworkSettings", {}).get("Ports", {})
    binding = ports.get("8081/tcp")
    if (not isinstance(binding, list) or len(binding) != 1
            or binding[0].get("HostIp") != "127.0.0.1"
            or any(value for key, value in ports.items() if key != "8081/tcp")):
        raise ValueError("smoke must publish only one loopback HTTP port")
    port = int(binding[0]["HostPort"])
    if not 1 <= port <= 65535:
        raise ValueError("invalid published HTTP port")
    return f"http://127.0.0.1:{port}"


def smoke(image, revision):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("revision must be a complete Git SHA")
    token = uuid.uuid4().hex
    prefix = "gelabber-media-smoke-" + token
    network = prefix + "-net"
    containers = []
    network_id = None
    report = {"schema": 1, "status": "FAIL", "revision": revision,
              "ready": False, "normal_shutdown": False, "forced_cleanup": False,
              "owner_token": token, "resources": {},
              "cleanup": {"complete": False, "errors": [], "verified_absent": []},
              "scope": "actual_image_worker_redis_readiness_and_shutdown_no_rtp"}
    try:
        media_image = inspect("image", image)
        if media_image is None or media_image.get("Config", {}).get("Labels", {}).get(
                "org.opencontainers.image.revision") != revision:
            raise ValueError("media image revision does not match requested source")
        report["image_id"] = media_image["Id"]
        redis_image = inspect("image", REDIS_IMAGE)
        if redis_image is None:
            docker("pull", REDIS_IMAGE, timeout=120)
            redis_image = inspect("image", REDIS_IMAGE)
        if redis_image is None:
            raise ValueError("pinned Redis image unavailable")
        # Register names before creation so even a timed-out Docker create is
        # recovered by its fresh nonce label, exact image and subsequent ID fence.
        network_id = docker("network", "create", "--label", f"{OWNER_LABEL}={token}", network)
        owned_network(inspect("network", network), name=network, token=token, identity=network_id)
        report["resources"]["network"] = {"id": network_id, "name": network}
        redis = {"name": prefix + "-redis", "image": redis_image["Id"], "id": None}
        containers.append(redis)
        redis["id"] = docker("run", "-d", "--name", redis["name"], "--network", network,
                             "--label", f"{OWNER_LABEL}={token}", redis["image"])
        owned_container(inspect("container", redis["name"]), name=redis["name"],
                        image=redis["image"], token=token, identity=redis["id"])
        report["resources"]["redis"] = {"id": redis["id"], "name": redis["name"], "image_id": redis["image"]}
        candidate = {"name": prefix + "-media", "image": media_image["Id"], "id": None}
        containers.append(candidate)
        candidate["id"] = docker(
            "run", "-d", "--name", candidate["name"], "--network", network,
            "--label", f"{OWNER_LABEL}={token}", "--publish", "127.0.0.1::8081",
            "--env", "MEDIA_ADDR=0.0.0.0:8081", "--env", f"REDIS_URL=redis://{redis['name']}:6379",
            "--env", "MEDIA_ICE_BIND=0.0.0.0:10000", "--env", "MEDIA_ICE_PORT_MAX=10031",
            "--env", "MEDIA_ADVERTISED_IP=127.0.0.1", candidate["image"])
        record = inspect("container", candidate["name"])
        owned_container(record, name=candidate["name"], image=candidate["image"],
                        token=token, identity=candidate["id"])
        address = http_address(record)
        report["resources"]["media"] = {"id": candidate["id"], "name": candidate["name"],
                                         "image_id": candidate["image"],
                                         "http_binding": record["NetworkSettings"]["Ports"]["8081/tcp"]}
        deadline = time.monotonic() + 60
        while True:
            record = inspect("container", candidate["id"])
            owned_container(record, name=candidate["name"], image=candidate["image"],
                            token=token, identity=candidate["id"])
            if not record["State"]["Running"]:
                raise ValueError("media exited before real readiness")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("media Redis/worker readiness exceeded 60 seconds")
            try:
                # Ignore host proxy variables for our explicit loopback binding.
                opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                with opener.open(address + "/ready", timeout=min(2, remaining)) as response:
                    payload = json.loads(response.read(65537))
                    if response.status == 200 and ready_payload(payload):
                        if time.monotonic() >= deadline:
                            raise TimeoutError("media readiness arrived after deadline")
                        report["ready"] = True
                        report["ready_checks"] = {name: payload["checks"][name]["status"]
                                                  for name in ("redis", "media")}
                        break
            except (urllib.error.URLError, TimeoutError, ValueError):
                pass
            time.sleep(min(0.2, max(0, deadline - time.monotonic())))
        docker("kill", "--signal", "TERM", candidate["id"])
        deadline = time.monotonic() + 20
        while True:
            record = inspect("container", candidate["id"])
            owned_container(record, name=candidate["name"], image=candidate["image"],
                            token=token, identity=candidate["id"])
            state = record["State"]
            if time.monotonic() >= deadline:
                raise TimeoutError("normal media shutdown exceeded 20 seconds")
            if not state["Running"]:
                report["exit_code"] = state["ExitCode"]
                if state["Status"] != "exited" or state["ExitCode"] != 0 or state.get("OOMKilled"):
                    raise ValueError("media did not exit normally after SIGTERM")
                report["normal_shutdown"] = True
                break
            time.sleep(0.2)
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        for resource in reversed(containers):
            try:
                record = inspect("container", resource["name"])
                if record is None:
                    continue
                identity = owned_container(record, name=resource["name"], image=resource["image"],
                                           token=token, identity=resource["id"])
                if record["State"]["Running"]:
                    if resource["name"].endswith("-media"):
                        report["forced_cleanup"] = True
                    else:
                        try:
                            docker("stop", "--time", "8", identity)
                        except Exception as error:
                            report["cleanup"]["errors"].append(f"Redis stop: {type(error).__name__}: {error}")
                # Recheck the exact ID/label/image before removing. --force
                # also closes resources when a previous stop request timed out.
                owned_container(inspect("container", identity), name=resource["name"],
                                image=resource["image"], token=token, identity=identity)
                docker("rm", "--force", identity)
                if inspect("container", identity) is not None:
                    raise ValueError("owned container remains after cleanup")
                report["cleanup"]["verified_absent"].append(identity)
            except Exception as error:
                report["cleanup"]["errors"].append(f"{resource['name']}: {type(error).__name__}: {error}")
        try:
            record = inspect("network", network)
            if record is not None:
                identity = owned_network(record, name=network, token=token, identity=network_id)
                if record.get("Containers"):
                    raise ValueError("owned network still has attached containers")
                docker("network", "rm", identity)
                if inspect("network", identity) is not None:
                    raise ValueError("owned network remains after cleanup")
                report["cleanup"]["verified_absent"].append(identity)
        except Exception as error:
            report["cleanup"]["errors"].append(f"network: {type(error).__name__}: {error}")
        report["cleanup"]["complete"] = not report["cleanup"]["errors"]
    if (report["ready"] and report["normal_shutdown"] and report["cleanup"]["complete"]
            and not report["forced_cleanup"] and "error" not in report):
        report["status"] = "PASS"
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    report = smoke(args.image, args.revision)
    print(json.dumps(report, indent=2))
    raise SystemExit(0 if report["status"] == "PASS" else 1)


if __name__ == "__main__":
    main()
