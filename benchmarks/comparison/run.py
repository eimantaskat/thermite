"""Loads the same app in thermite, axum and FastAPI with oha and prints markdown tables.

Run on Linux, from this directory:

    cargo build --release --example comparison
    cargo build --release --manifest-path axum/Cargo.toml
    python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
    python3 run.py --thermite ../../target/release/examples/comparison \
        --axum axum/target/release/axum-comparison --python .venv/bin/python

Any of the three can be left out, as long as two remain; the first given is the one the others are compared with.
Each server is pinned to its own cores with taskset and oha to others, so that they do not compete.
All servers first answer every case once, and the run stops if any answer differs.
"""

import argparse
import json
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

HERE = Path(__file__).resolve().parent
PORT = 8765


@dataclass
class Case:
    name: str
    method: str
    path: str
    body: object = None


def item(number):
    return {"name": f"item {number}", "price": 9.5 + number, "tags": ["new", "sale"]}


CASES = [
    Case("plaintext", "GET", "/plaintext"),
    Case("json", "GET", "/json"),
    Case("path and query", "GET", "/items/42?q=hello&limit=5"),
    Case("small body", "POST", "/items", item(1)),
    Case("100 items", "POST", "/items/batch", [item(number) for number in range(100)]),
    Case("invalid body (422)", "POST", "/items", {"name": "item", "price": "free", "tags": [1]}),
]


@dataclass
class Server:
    name: str
    command: list
    workers: int


def servers(arguments, cores, workers):
    pinned = ["taskset", "-c", cores]
    commands = []
    for name, binary in (("thermite", arguments.thermite), ("axum", arguments.axum)):
        if binary:
            commands.append((name, [binary, f"127.0.0.1:{PORT}", str(workers)]))
    if arguments.python:
        commands.append(("FastAPI", [
            arguments.python, "-m", "uvicorn", "fastapi_app:app",
            "--host", "127.0.0.1", "--port", str(PORT), "--workers", str(workers),
            "--loop", "uvloop", "--http", "httptools", "--no-access-log", "--log-level", "warning",
        ]))
    return [Server(name, pinned + command, workers) for name, command in commands]


def start(server):
    process = subprocess.Popen(server.command, cwd=HERE)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            sys.exit(f"{server.name} exited with {process.returncode}")
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{PORT}/plaintext", timeout=1).read()
            return process
        except (urllib.error.URLError, ConnectionError):
            time.sleep(0.2)
    stop(process)
    sys.exit(f"{server.name} did not answer within 30 s")


def stop(process):
    process.send_signal(signal.SIGINT)
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def answer(case):
    data = None if case.body is None else json.dumps(case.body).encode()
    request = urllib.request.Request(
        f"http://127.0.0.1:{PORT}{case.path}", data=data, method=case.method,
        headers={"content-type": "application/json"} if data else {},
    )
    try:
        response = urllib.request.urlopen(request, timeout=5)
    except urllib.error.HTTPError as error:
        response = error
    content_type = response.headers.get("content-type", "")
    body = response.read()
    if content_type.startswith("application/json"):
        body = json.loads(body)
    return response.status, content_type.replace(" ", "").lower(), body


def load(case, arguments, body_file, duration):
    command = [
        "taskset", "-c", arguments.load_cores, "oha", "--no-tui", "--output-format", "json", "-w",
        "-z", f"{duration}s", "-c", str(arguments.connections), "-m", case.method,
    ]
    if body_file:
        command += ["-T", "application/json", "-D", body_file]
    command.append(f"http://127.0.0.1:{PORT}{case.path}")
    return json.loads(subprocess.run(command, check=True, capture_output=True, text=True).stdout)


def measure(server, arguments, body_files, expected):
    process = start(server)
    try:
        results = {}
        for case in CASES:
            body_file = body_files.get(case.name)
            load(case, arguments, body_file, arguments.warmup)
            report = load(case, arguments, body_file, arguments.duration)
            statuses = report["statusCodeDistribution"]
            if set(statuses) != {str(expected[case.name])} or report["errorDistribution"]:
                sys.exit(f"{server.name} {case.name}: {statuses} {report['errorDistribution']}")
            results[case.name] = report
            print(f"  {server.name} {case.name}: {report['summary']['requestsPerSec']:,.0f} req/s", file=sys.stderr)
        return results
    finally:
        stop(process)


def check_same_answers(all_servers):
    answers = {}
    for server in all_servers:
        process = start(server)
        try:
            answers[server.name] = {case.name: answer(case) for case in CASES}
        finally:
            stop(process)
    reference, *others = all_servers
    expected = answers[reference.name]
    differs = False
    for other in others:
        for case in CASES:
            if answers[other.name][case.name] != expected[case.name]:
                differs = True
                print(
                    f"{case.name}:\n  {reference.name} {expected[case.name]}\n  {other.name} {answers[other.name][case.name]}",
                    file=sys.stderr,
                )
    if differs:
        sys.exit("the servers answer differently, so they would not be doing the same work")
    return {name: status for name, (status, _, _) in expected.items()}


def milliseconds(report, percentile):
    return f"{report['latencyPercentiles'][percentile] * 1000:.2f}"


def rate(report):
    return report["summary"]["requestsPerSec"]


def ratio(ours, theirs):
    # near 1 a single decimal hides differences the runs can tell apart
    value = ours / theirs
    return f"{value:.2f}×" if value < 1.5 else f"{value:.1f}×"


def tables(title, results):
    names = list(results)
    reference, others = names[0], names[1:]
    throughput = [
        f"### {title}: requests per second",
        "",
        "| Case | " + " | ".join(names) + " | " + " | ".join(f"{reference} ÷ {name}" for name in others) + " |",
        "|---|" + "---:|" * (len(names) + len(others)),
    ]
    latency = [
        f"### {title}: latency p50 / p99 ms",
        "",
        "| Case | " + " | ".join(names) + " |",
        "|---|" + "---:|" * len(names),
    ]
    for case in CASES:
        reports = [results[name][case.name] for name in names]
        rates = [f"{rate(report):,.0f}" for report in reports]
        ratios = [ratio(rate(reports[0]), rate(report)) for report in reports[1:]]
        throughput.append(f"| {case.name} | " + " | ".join(rates + ratios) + " |")
        latencies = [f"{milliseconds(report, 'p50')} / {milliseconds(report, 'p99')}" for report in reports]
        latency.append(f"| {case.name} | " + " | ".join(latencies) + " |")
    return "\n".join(throughput) + "\n\n" + "\n".join(latency)


def core_count(cores):
    count = 0
    for part in cores.split(","):
        first, _, last = part.partition("-")
        count += int(last or first) - int(first) + 1
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--thermite", help="the built comparison example")
    parser.add_argument("--axum", help="the built axum-comparison binary")
    parser.add_argument("--python", help="a Python with requirements.txt installed, for FastAPI")
    parser.add_argument("--cores", default="0-7", help="CPUs for the multi-core run, as taskset takes them")
    parser.add_argument("--load-cores", default="8-15", help="cores for oha")
    parser.add_argument("--connections", type=int, default=64)
    parser.add_argument("--duration", type=int, default=10, help="seconds measured per case")
    parser.add_argument("--warmup", type=int, default=2, help="seconds of load before each measurement")
    parser.add_argument("--output", type=Path, help="a file for oha's raw reports")
    arguments = parser.parse_args()
    if sum(bool(server) for server in (arguments.thermite, arguments.axum, arguments.python)) < 2:
        parser.error("give at least two of --thermite, --axum and --python")

    first_core = arguments.cores.split(",")[0].split("-")[0]
    runs = [("One core", first_core, 1), (f"CPUs {arguments.cores}", arguments.cores, core_count(arguments.cores))]
    expected = check_same_answers(servers(arguments, first_core, 1))

    with tempfile.TemporaryDirectory() as directory:
        body_files = {}
        for case in CASES:
            if case.body is not None:
                path = Path(directory) / f"{len(body_files)}.json"
                path.write_text(json.dumps(case.body))
                body_files[case.name] = str(path)

        reports, printed = {}, []
        for title, cores, workers in runs:
            print(f"{title}:", file=sys.stderr)
            results = {server.name: measure(server, arguments, body_files, expected) for server in servers(arguments, cores, workers)}
            reports[title] = results
            printed.append(tables(title, results))

    if arguments.output:
        arguments.output.write_text(json.dumps(reports, indent=2))
    print("\n\n".join(printed))


if __name__ == "__main__":
    main()
