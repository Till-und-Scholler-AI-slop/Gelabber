#!/usr/bin/env python3
"""Parse Chrome chrome://webrtc-internals PeerConnection dump JSON.

Extracts measured codec / bitrate / loss / jitter / concealment / audio level
fields. Prints N/A when a field is absent. Never invents rates from a single
snapshot.

Usage:
  python3 scripts/parse-webrtc-dump.py path/to/dump.json
  python3 scripts/parse-webrtc-dump.py --help
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from typing import Any


NA = "N/A"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="parse-webrtc-dump.py",
        description=(
            "Parse a Chrome webrtc-internals dump "
            '("Download the PeerConnection updates and stats data") '
            "and print codec, bitrate, packetsLost, jitter, concealment, "
            "and audioLevel fields when present."
        ),
    )
    p.add_argument(
        "dump",
        nargs="?",
        help="Path to webrtc-internals JSON dump",
    )
    return p.parse_args(argv)


def load_json(path: str) -> Any:
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def parse_values(raw: Any) -> list[Any]:
    """Chrome often stores values as a JSON-encoded string array."""
    if raw is None:
        return []
    if isinstance(raw, list):
        return raw
    if isinstance(raw, str):
        s = raw.strip()
        if not s:
            return []
        try:
            parsed = json.loads(s)
        except json.JSONDecodeError:
            return [raw]
        if isinstance(parsed, list):
            return parsed
        return [parsed]
    return [raw]


def parse_iso_ms(s: Any) -> float | None:
    if not isinstance(s, str) or not s:
        return None
    try:
        # Chrome uses ...Z UTC
        if s.endswith("Z"):
            s = s[:-1] + "+00:00"
        return datetime.fromisoformat(s).timestamp() * 1000.0
    except ValueError:
        return None


def last_present(values: list[Any]) -> Any:
    for v in reversed(values):
        if v is None:
            continue
        if isinstance(v, float) and math.isnan(v):
            continue
        return v
    return None


def as_number(v: Any) -> float | None:
    if isinstance(v, bool):
        return None
    if isinstance(v, (int, float)):
        if isinstance(v, float) and math.isnan(v):
            return None
        return float(v)
    if isinstance(v, str):
        try:
            return float(v)
        except ValueError:
            return None
    return None


def fmt_num(v: Any, digits: int = 3) -> str:
    n = as_number(v)
    if n is None:
        return NA
    if abs(n - round(n)) < 1e-9:
        return str(int(round(n)))
    return f"{n:.{digits}f}".rstrip("0").rstrip(".")


def fmt_kbps(bits_per_sec: float | None) -> str:
    if bits_per_sec is None:
        return NA
    kbps = bits_per_sec / 1000.0
    return f"{kbps:.2f} kbps"


def split_stat_key(key: str) -> tuple[str, str] | None:
    """Split 'ReportId-fieldName' on the first hyphen."""
    if "-" not in key:
        return None
    rid, field = key.split("-", 1)
    if not rid or not field:
        return None
    return rid, field


def index_chrome_stats(stats_obj: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """Group chrome dump series by report id.

    Each entry becomes:
      {
        "type": statsType or None,
        "startTime": ...,
        "endTime": ...,
        "fields": { fieldName: [samples...] }
      }
    """
    reports: dict[str, dict[str, Any]] = {}
    for key, entry in stats_obj.items():
        if not isinstance(entry, dict):
            continue
        parts = split_stat_key(key)
        if not parts:
            continue
        rid, field = parts
        rep = reports.setdefault(
            rid,
            {"type": None, "startTime": None, "endTime": None, "fields": {}},
        )
        st = entry.get("statsType")
        if isinstance(st, str) and st:
            rep["type"] = st
        if rep["startTime"] is None and entry.get("startTime") is not None:
            rep["startTime"] = entry.get("startTime")
        if entry.get("endTime") is not None:
            rep["endTime"] = entry.get("endTime")
        rep["fields"][field] = parse_values(entry.get("values"))
    return reports


def snapshots_from_array(stats_list: list[Any]) -> list[dict[str, Any]]:
    """Normalize alternate dump shapes into [{timestamp_ms, reports}].

    Accepts:
      - [{timestamp, values: {id: {type, ...fields}}}]
      - [{timestamp, reports: {...}}]
      - list of getStats-like dict maps {id: report}
    """
    out: list[dict[str, Any]] = []
    for item in stats_list:
        if not isinstance(item, dict):
            continue
        ts = item.get("timestamp")
        if ts is None:
            ts = item.get("time")
        ts_ms = as_number(ts)
        reports: dict[str, dict[str, Any]] = {}

        payload = item.get("values")
        if payload is None:
            payload = item.get("reports")
        if payload is None and all(
            isinstance(v, dict) and ("type" in v or "id" in v) for v in item.values()
            if not isinstance(v, (str, int, float, bool, type(None)))
        ):
            # Bare map of id -> report (excluding timestamp keys already handled)
            payload = {
                k: v
                for k, v in item.items()
                if k not in ("timestamp", "time") and isinstance(v, dict)
            }

        if isinstance(payload, dict):
            for rid, report in payload.items():
                if not isinstance(report, dict):
                    continue
                fields = {
                    k: v
                    for k, v in report.items()
                    if k not in ("id", "type", "timestamp")
                }
                reports[str(rid)] = {
                    "type": report.get("type"),
                    "fields": fields,
                    "timestamp": report.get("timestamp", ts),
                }
        if reports:
            out.append({"timestamp_ms": ts_ms, "reports": reports})
    return out


def reports_from_pc(pc: dict[str, Any]) -> tuple[str, Any]:
    """Return ('chrome-series'|'snapshots'|'empty', data)."""
    stats = pc.get("stats")
    if stats is None:
        return "empty", None
    if isinstance(stats, dict):
        # Could be chrome series map OR a single snapshot map of id->report
        sample = next(iter(stats.values()), None)
        if isinstance(sample, dict) and (
            "values" in sample or "statsType" in sample or "startTime" in sample
        ):
            return "chrome-series", index_chrome_stats(stats)
        # Maybe already id -> flat report
        if sample is not None and isinstance(sample, dict) and "type" in sample:
            return "snapshots", snapshots_from_array([{"timestamp": None, "values": stats}])
        return "chrome-series", index_chrome_stats(stats)
    if isinstance(stats, list):
        return "snapshots", snapshots_from_array(stats)
    return "empty", None


def field_last(report: dict[str, Any], name: str) -> Any:
    fields = report.get("fields") or {}
    vals = fields.get(name)
    if vals is None:
        return None
    if isinstance(vals, list):
        return last_present(vals)
    return vals


def field_series(report: dict[str, Any], name: str) -> list[Any]:
    fields = report.get("fields") or {}
    vals = fields.get(name)
    if vals is None:
        return []
    if isinstance(vals, list):
        return vals
    return [vals]


def bitrate_from_bytes(
    bytes_series: list[Any],
    start_time: Any,
    end_time: Any,
) -> tuple[str, str]:
    """Return (bitrate_line, note). Prefer delta over last two samples.

    If only one sample: report cumulative bytes, no invented rate.
    """
    nums = [as_number(v) for v in bytes_series]
    nums = [n for n in nums if n is not None]
    if not nums:
        return NA, "no bytes samples"
    if len(nums) == 1:
        return (
            NA,
            f"single snapshot only; cumulative bytes={fmt_num(nums[0])}",
        )

    # Prefer last two distinct cumulative values with inferred interval
    b0, b1 = nums[-2], nums[-1]
    delta_b = b1 - b0
    t0 = parse_iso_ms(start_time)
    t1 = parse_iso_ms(end_time)
    if t0 is not None and t1 is not None and t1 > t0 and len(nums) >= 2:
        # Evenly spaced samples across [start, end] is Chrome's usual shape.
        interval_ms = (t1 - t0) / (len(nums) - 1)
    else:
        interval_ms = None

    if interval_ms is not None and interval_ms > 0:
        bps = (delta_b * 8.0) * (1000.0 / interval_ms)
        return (
            fmt_kbps(bps),
            f"from Δbytes={fmt_num(delta_b)} over ~{fmt_num(interval_ms)} ms "
            f"(last interval; n={len(nums)} samples)",
        )

    # Fallback: whole-span average if times exist
    if t0 is not None and t1 is not None and t1 > t0:
        span_ms = t1 - t0
        total_delta = nums[-1] - nums[0]
        bps = (total_delta * 8.0) * (1000.0 / span_ms)
        return (
            fmt_kbps(bps),
            f"from Δbytes={fmt_num(total_delta)} over {fmt_num(span_ms)} ms span "
            f"(n={len(nums)} samples; no per-sample timestamps)",
        )

    return (
        NA,
        f"cannot compute rate (missing timestamps); "
        f"cumulative bytes first={fmt_num(nums[0])} last={fmt_num(nums[-1])}",
    )


def bitrate_from_snapshot_pair(
    earlier: dict[str, Any] | None,
    later: dict[str, Any] | None,
    byte_field: str,
) -> tuple[str, str]:
    if later is None:
        return NA, "no report"
    b1 = as_number(later.get("fields", {}).get(byte_field))
    if earlier is None or b1 is None:
        if b1 is not None:
            return NA, f"single snapshot only; cumulative bytes={fmt_num(b1)}"
        return NA, "no bytes"
    b0 = as_number(earlier.get("fields", {}).get(byte_field))
    t0 = as_number(earlier.get("timestamp"))
    t1 = as_number(later.get("timestamp"))
    if b0 is None:
        return NA, f"single snapshot only; cumulative bytes={fmt_num(b1)}"
    if t0 is None or t1 is None or t1 <= t0:
        return (
            NA,
            f"cannot compute rate (bad timestamps); "
            f"cumulative bytes first={fmt_num(b0)} last={fmt_num(b1)}",
        )
    # timestamps may be ms or us; Chrome getStats timestamp is DOMHighRes ms
    dt_ms = t1 - t0
    if dt_ms > 1e10:  # likely microseconds
        dt_ms = dt_ms / 1000.0
    bps = ((b1 - b0) * 8.0) * (1000.0 / dt_ms)
    return (
        fmt_kbps(bps),
        f"from Δbytes={fmt_num(b1 - b0)} over {fmt_num(dt_ms)} ms",
    )


def find_codec_for(
    reports: dict[str, dict[str, Any]],
    codec_id: Any,
) -> dict[str, Any] | None:
    if not codec_id:
        return None
    cid = str(codec_id)
    if cid in reports and (reports[cid].get("type") in (None, "codec") or True):
        rep = reports[cid]
        if rep.get("type") in (None, "codec") or field_last(rep, "mimeType") is not None:
            return rep
    # Sometimes codecId is the full id already
    for rid, rep in reports.items():
        if rid == cid or rid.endswith(cid):
            if rep.get("type") == "codec" or field_last(rep, "mimeType") is not None:
                return rep
    return None


def codec_line(reports: dict[str, dict[str, Any]], rtp_report: dict[str, Any]) -> str:
    codec_id = field_last(rtp_report, "codecId")
    codec = find_codec_for(reports, codec_id)
    mime = field_last(codec, "mimeType") if codec else None
    fmtp = field_last(codec, "sdpFmtpLine") if codec else None
    if mime is None and fmtp is None:
        # Sometimes mimeType appears directly on rtp in odd dumps
        mime = field_last(rtp_report, "mimeType")
        fmtp = field_last(rtp_report, "sdpFmtpLine")
    if mime is None and fmtp is None:
        return NA
    mime_s = str(mime) if mime is not None else NA
    fmtp_s = str(fmtp) if fmtp is not None else NA
    return f"{mime_s} | fmtp={fmtp_s}"


def kind_of(report: dict[str, Any]) -> str:
    k = field_last(report, "kind")
    if k:
        return str(k)
    mid = field_last(report, "mediaType")
    if mid:
        return str(mid)
    return "?"


def print_rtp_chrome(
    direction: str,
    rid: str,
    report: dict[str, Any],
    all_reports: dict[str, dict[str, Any]],
) -> None:
    kind = kind_of(report)
    print(f"  [{direction}] id={rid} kind={kind}")
    print(f"    active codec:     {codec_line(all_reports, report)}")

    byte_field = "bytesSent" if direction == "outbound-rtp" else "bytesReceived"
    series = field_series(report, byte_field)
    rate, note = bitrate_from_bytes(series, report.get("startTime"), report.get("endTime"))
    # If chrome already computed bits/s, mention it as secondary measured series
    derived_key = "[bytesSent_in_bits/s]" if direction == "outbound-rtp" else "[bytesReceived_in_bits/s]"
    derived = field_series(report, derived_key)
    derived_last = as_number(last_present(derived)) if derived else None

    print(f"    bitrate:          {rate}")
    print(f"                      ({note})")
    if derived_last is not None:
        print(f"    bitrate (chrome [bytes_*_in_bits/s] last): {fmt_kbps(derived_last)}")

    print(f"    packetsLost:      {fmt_num(field_last(report, 'packetsLost'))}")
    print(f"    jitter:           {fmt_num(field_last(report, 'jitter'))} s")

    if direction == "inbound-rtp" and kind == "audio":
        print(f"    concealedSamples:       {fmt_num(field_last(report, 'concealedSamples'))}")
        print(
            f"    silentConcealedSamples: {fmt_num(field_last(report, 'silentConcealedSamples'))}"
        )
        print(f"    concealmentEvents:      {fmt_num(field_last(report, 'concealmentEvents'))}")
    elif direction == "inbound-rtp":
        # Still show if present on non-audio
        for name in ("concealedSamples", "silentConcealedSamples", "concealmentEvents"):
            v = field_last(report, name)
            if v is not None:
                print(f"    {name}: {fmt_num(v)}")

    # audio level-ish on rtp itself (rare)
    for name in ("audioLevel", "totalAudioEnergy", "totalSamplesDuration"):
        v = field_last(report, name)
        if v is not None:
            print(f"    {name}: {fmt_num(v)}")


def print_level_report(label: str, rid: str, report: dict[str, Any]) -> None:
    al = field_last(report, "audioLevel")
    energy = field_last(report, "totalAudioEnergy")
    dur = field_last(report, "totalSamplesDuration")
    if al is None and energy is None and dur is None:
        return
    print(f"  [{label}] id={rid}")
    print(f"    audioLevel:           {fmt_num(al)}")
    print(f"    totalAudioEnergy:     {fmt_num(energy)}")
    print(f"    totalSamplesDuration: {fmt_num(dur)}")


def process_chrome_series(pc_id: str, reports: dict[str, dict[str, Any]]) -> None:
    print(f"PeerConnection: {pc_id}")
    print(f"  reports indexed: {len(reports)}")

    rtp_types = ("outbound-rtp", "inbound-rtp", "remote-inbound-rtp", "remote-outbound-rtp")
    any_rtp = False
    for rid, rep in sorted(reports.items(), key=lambda x: (str(x[1].get("type")), x[0])):
        rtype = rep.get("type")
        if rtype in ("outbound-rtp", "inbound-rtp"):
            any_rtp = True
            print_rtp_chrome(str(rtype), rid, rep, reports)
        elif rtype == "remote-inbound-rtp":
            any_rtp = True
            print(f"  [remote-inbound-rtp] id={rid} kind={kind_of(rep)}")
            print(f"    packetsLost: {fmt_num(field_last(rep, 'packetsLost'))}")
            print(f"    jitter:      {fmt_num(field_last(rep, 'jitter'))} s")
            for name in ("roundTripTime", "fractionLost"):
                v = field_last(rep, name)
                if v is not None:
                    print(f"    {name}: {fmt_num(v)}")
            for name in ("audioLevel", "totalAudioEnergy", "totalSamplesDuration"):
                v = field_last(rep, name)
                if v is not None:
                    print(f"    {name}: {fmt_num(v)}")

    if not any_rtp:
        print("  (no outbound-rtp / inbound-rtp reports found)")

    for rid, rep in sorted(reports.items()):
        rtype = rep.get("type")
        if rtype in ("media-source", "track", "remote-inbound-rtp"):
            # remote-inbound already printed above; still allow level block if only levels
            if rtype == "remote-inbound-rtp":
                continue
            print_level_report(str(rtype), rid, rep)
        elif rtype is None:
            # untyped but has audioLevel
            print_level_report("untyped", rid, rep)

    print()


def flatten_snapshot_reports(
    snaps: list[dict[str, Any]],
) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
    """Return (latest_by_id, previous_by_id, latest_meta) for snapshot mode."""
    if not snaps:
        return {}, {}, {}
    latest = snaps[-1]["reports"]
    previous = snaps[-2]["reports"] if len(snaps) >= 2 else {}
    # Attach timestamps into report copies for bitrate helper
    def with_ts(src: dict[str, dict[str, Any]], ts: float | None) -> dict[str, dict[str, Any]]:
        out: dict[str, dict[str, Any]] = {}
        for rid, rep in src.items():
            out[rid] = {
                "type": rep.get("type"),
                "fields": dict(rep.get("fields") or {}),
                "timestamp": rep.get("timestamp", ts),
            }
        return out

    latest_ts = snaps[-1].get("timestamp_ms")
    prev_ts = snaps[-2].get("timestamp_ms") if len(snaps) >= 2 else None
    return with_ts(latest, latest_ts), with_ts(previous, prev_ts), {}


def process_snapshots(pc_id: str, snaps: list[dict[str, Any]]) -> None:
    print(f"PeerConnection: {pc_id}")
    print(f"  snapshots: {len(snaps)}")
    latest, previous, _ = flatten_snapshot_reports(snaps)
    if not latest:
        print("  (empty stats)")
        print()
        return

    # Build a chrome-like view for codec lookup using latest field values as 1-sample series
    chrome_like: dict[str, dict[str, Any]] = {}
    for rid, rep in latest.items():
        chrome_like[rid] = {
            "type": rep.get("type"),
            "startTime": None,
            "endTime": None,
            "fields": {k: [v] for k, v in (rep.get("fields") or {}).items()},
        }

    any_rtp = False
    for rid, rep in sorted(latest.items(), key=lambda x: (str(x[1].get("type")), x[0])):
        rtype = rep.get("type")
        if rtype not in ("outbound-rtp", "inbound-rtp"):
            continue
        any_rtp = True
        kind = str((rep.get("fields") or {}).get("kind") or "?")
        print(f"  [{rtype}] id={rid} kind={kind}")
        print(f"    active codec:     {codec_line(chrome_like, chrome_like[rid])}")
        byte_field = "bytesSent" if rtype == "outbound-rtp" else "bytesReceived"
        rate, note = bitrate_from_snapshot_pair(previous.get(rid), rep, byte_field)
        print(f"    bitrate:          {rate}")
        print(f"                      ({note})")
        fields = rep.get("fields") or {}
        print(f"    packetsLost:      {fmt_num(fields.get('packetsLost'))}")
        print(f"    jitter:           {fmt_num(fields.get('jitter'))} s")
        if rtype == "inbound-rtp":
            print(f"    concealedSamples:       {fmt_num(fields.get('concealedSamples'))}")
            print(
                f"    silentConcealedSamples: {fmt_num(fields.get('silentConcealedSamples'))}"
            )
            print(f"    concealmentEvents:      {fmt_num(fields.get('concealmentEvents'))}")
        for name in ("audioLevel", "totalAudioEnergy", "totalSamplesDuration"):
            if name in fields:
                print(f"    {name}: {fmt_num(fields.get(name))}")

    if not any_rtp:
        print("  (no outbound-rtp / inbound-rtp reports found)")

    for rid, rep in sorted(latest.items()):
        rtype = rep.get("type")
        if rtype in ("media-source", "track"):
            fields = rep.get("fields") or {}
            al, energy, dur = (
                fields.get("audioLevel"),
                fields.get("totalAudioEnergy"),
                fields.get("totalSamplesDuration"),
            )
            if al is None and energy is None and dur is None:
                continue
            print(f"  [{rtype}] id={rid}")
            print(f"    audioLevel:           {fmt_num(al)}")
            print(f"    totalAudioEnergy:     {fmt_num(energy)}")
            print(f"    totalSamplesDuration: {fmt_num(dur)}")
        elif rtype == "remote-inbound-rtp":
            fields = rep.get("fields") or {}
            print(f"  [remote-inbound-rtp] id={rid} kind={fields.get('kind', '?')}")
            print(f"    packetsLost: {fmt_num(fields.get('packetsLost'))}")
            print(f"    jitter:      {fmt_num(fields.get('jitter'))} s")
            for name in ("audioLevel", "totalAudioEnergy", "totalSamplesDuration"):
                if name in fields:
                    print(f"    {name}: {fmt_num(fields.get(name))}")
    print()


def iter_peer_connections(dump: Any) -> list[tuple[str, dict[str, Any]]]:
    if not isinstance(dump, dict):
        return []
    pcs = dump.get("PeerConnections")
    if isinstance(pcs, dict):
        return [(str(k), v) for k, v in pcs.items() if isinstance(v, dict)]
    # Some exports nest under "peerConnections" or are a bare PC map
    pcs = dump.get("peerConnections")
    if isinstance(pcs, dict):
        return [(str(k), v) for k, v in pcs.items() if isinstance(v, dict)]
    # Bare single PC with stats
    if "stats" in dump:
        return [("unknown", dump)]
    return []


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if not args.dump:
        print("error: dump path required (see --help)", file=sys.stderr)
        return 2
    try:
        dump = load_json(args.dump)
    except OSError as e:
        print(f"error: cannot read {args.dump}: {e}", file=sys.stderr)
        return 1
    except json.JSONDecodeError as e:
        print(f"error: invalid JSON in {args.dump}: {e}", file=sys.stderr)
        return 1

    pcs = iter_peer_connections(dump)
    if not pcs:
        print("No PeerConnections found in dump.")
        gum = dump.get("getUserMedia") if isinstance(dump, dict) else None
        if gum is not None:
            print(f"getUserMedia entries: {len(gum) if isinstance(gum, list) else NA}")
        return 0

    print(f"Dump: {args.dump}")
    print(f"PeerConnections: {len(pcs)}")
    print("---")

    for pc_id, pc in pcs:
        mode, data = reports_from_pc(pc)
        if mode == "chrome-series":
            process_chrome_series(pc_id, data)
        elif mode == "snapshots":
            process_snapshots(pc_id, data)
        else:
            print(f"PeerConnection: {pc_id}")
            print("  (no stats object)")
            print()

    print(
        "Note: fields printed as N/A were absent in the dump. "
        "No audio/product fixes without measured dump data."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
