"""Loads the same app in thermite and in FastAPI with oha and prints a markdown table.

Run on Linux, from this directory:

    cargo build --release --example fastapi_comparison
    python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
    python3 run.py --thermite ../../target/release/examples/fastapi_comparison --python .venv/bin/python

Each server is pinned to its own cores with taskset and oha to others, so that they do not compete.
Both servers first answer every case once, and the run stops if the answers differ.
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
    thermite = [arguments.thermite, f"127.0.0.1:{PORT}", str(workers)]
    uvicorn = [
        arguments.python, "-m", "uvicorn", "app:app",
        "--host", "127.0.0.1", "--port", str(PORT), "--workers", str(workers),
        "--loop", "uvloop", "--http", "httptools", "--no-access-log", "--log-level", "warning",
    ]
    return [Server("thermite", pinned + thermite, workers), Server("FastAPI", pinned + uvicorn, workers)]


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


def check_same_answers(pair):
    answers = []
    for server in pair:
        process = start(server)
        try:
            answers.append({case.name: answer(case) for case in CASES})
        finally:
            stop(process)
    thermite, fastapi = answers
    differences = [name for name in thermite if thermite[name] != fastapi[name]]
    for name in differences:
        print(f"{name}:\n  thermite {thermite[name]}\n  FastAPI  {fastapi[name]}", file=sys.stderr)
    if differences:
        sys.exit("the servers answer differently, so they would not be doing the same work")
    return {name: status for name, (status, _, _) in thermite.items()}


def milliseconds(report, percentile):
    return f"{report['latencyPercentiles'][percentile] * 1000:.2f}"


def table(title, results):
    thermite, fastapi = results["thermite"], results["FastAPI"]
    lines = [
        f"### {title}",
        "",
        "| Case | thermite req/s | FastAPI req/s | Ratio | thermite p50 / p99 ms | FastAPI p50 / p99 ms |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for case in CASES:
        ours, theirs = thermite[case.name], fastapi[case.name]
        ours_rate, theirs_rate = ours["summary"]["requestsPerSec"], theirs["summary"]["requestsPerSec"]
        lines.append(
            f"| {case.name} | {ours_rate:,.0f} | {theirs_rate:,.0f} | {ours_rate / theirs_rate:.1f}× "
            f"| {milliseconds(ours, 'p50')} / {milliseconds(ours, 'p99')} "
            f"| {milliseconds(theirs, 'p50')} / {milliseconds(theirs, 'p99')} |"
        )
    return "\n".join(lines)


def core_count(cores):
    count = 0
    for part in cores.split(","):
        first, _, last = part.partition("-")
        count += int(last or first) - int(first) + 1
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--thermite", required=True, help="the built fastapi_comparison example")
    parser.add_argument("--python", required=True, help="a Python with requirements.txt installed")
    parser.add_argument("--cores", default="0-3", help="cores for the multi-core run, as taskset takes them")
    parser.add_argument("--load-cores", default="8-15", help="cores for oha")
    parser.add_argument("--connections", type=int, default=64)
    parser.add_argument("--duration", type=int, default=10, help="seconds measured per case")
    parser.add_argument("--warmup", type=int, default=2, help="seconds of load before each measurement")
    parser.add_argument("--output", type=Path, help="a file for oha's raw reports")
    arguments = parser.parse_args()

    first_core = arguments.cores.split(",")[0].split("-")[0]
    runs = [("One core", first_core, 1), (f"{core_count(arguments.cores)} cores", arguments.cores, core_count(arguments.cores))]
    expected = check_same_answers(servers(arguments, first_core, 1))

    with tempfile.TemporaryDirectory() as directory:
        body_files = {}
        for case in CASES:
            if case.body is not None:
                path = Path(directory) / f"{len(body_files)}.json"
                path.write_text(json.dumps(case.body))
                body_files[case.name] = str(path)

        reports, tables = {}, []
        for title, cores, workers in runs:
            print(f"{title}:", file=sys.stderr)
            results = {server.name: measure(server, arguments, body_files, expected) for server in servers(arguments, cores, workers)}
            reports[title] = results
            tables.append(table(title, results))

    if arguments.output:
        arguments.output.write_text(json.dumps(reports, indent=2))
    print("\n\n".join(tables))


if __name__ == "__main__":
    main()
