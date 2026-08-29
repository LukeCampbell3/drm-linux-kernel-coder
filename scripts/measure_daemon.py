#!/usr/bin/env python3
import argparse
import json
import os
import statistics
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument("--binary", required=True)
parser.add_argument("--socket", required=True)
parser.add_argument("--work", required=True)
parser.add_argument("--count", type=int, default=200)
parser.add_argument("--output", required=True)
args = parser.parse_args()

started = time.monotonic_ns()
daemon = subprocess.Popen([args.binary, "serve", "--socket", args.socket, "--work", args.work],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    deadline = time.monotonic() + 10
    while not os.path.exists(args.socket):
        if daemon.poll() is not None:
            raise SystemExit("daemon exited during startup")
        if time.monotonic() >= deadline:
            raise SystemExit("daemon startup timed out")
        time.sleep(0.005)
    startup_ms = (time.monotonic_ns() - started) / 1_000_000
    rss_kb = int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(daemon.pid)], text=True).strip())
    samples = []
    begin = time.monotonic_ns()
    for index in range(args.count):
        call = time.monotonic_ns()
        result = subprocess.run([
            args.binary, "submit", "--socket", args.socket, "--task", f"baseline_{index % 8}",
            "--ops", "timer.observe"
        ], capture_output=True, text=True, check=False)
        if result.returncode != 0:
            raise SystemExit(result.stderr)
        samples.append((time.monotonic_ns() - call) / 1_000_000)
    elapsed_s = (time.monotonic_ns() - begin) / 1_000_000_000
    active_rss_kb = int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(daemon.pid)], text=True).strip())
    samples.sort()
    percentile = lambda p: samples[min(len(samples) - 1, max(0, (p * len(samples) + 99) // 100 - 1))]
    report = {
        "schema": 1,
        "startup_ms": round(startup_ms, 3),
        "idle_rss_kb": rss_kb,
        "active_rss_kb": active_rss_kb,
        "submissions": len(samples),
        "throughput_per_second": round(len(samples) / elapsed_s, 3),
        "submission_latency_ms": {
            "p50": round(statistics.median(samples), 3),
            "p95": round(percentile(95), 3),
            "p99": round(percentile(99), 3),
        },
    }
    with open(args.output, "w", encoding="utf-8") as target:
        json.dump(report, target, indent=2, sort_keys=True)
        target.write("\n")
finally:
    daemon.terminate()
    try:
        daemon.wait(timeout=5)
    except subprocess.TimeoutExpired:
        daemon.kill()
        daemon.wait()
