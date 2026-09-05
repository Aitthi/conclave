#!/usr/bin/env python3
"""Record a scripted interactive CLI session from a real pseudo-terminal.

The JSONL output preserves PTY reads and input/resize events in observation
order. Raw output bytes are base64 encoded so replay never passes through a
text decoder.
"""

import argparse
import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import shlex
import signal
import struct
import sys
import termios
import threading
import time


def parse_size(value: str) -> tuple[int, int]:
    match = re.fullmatch(r"(\d+)[xX](\d+)", value)
    if not match:
        raise argparse.ArgumentTypeError("expected COLSxROWS, for example 120x40")
    cols, rows = (int(match.group(1)), int(match.group(2)))
    if cols < 1 or rows < 1:
        raise argparse.ArgumentTypeError("columns and rows must be positive")
    return cols, rows


def parse_env(value: str) -> tuple[str, str]:
    if "=" not in value:
        raise argparse.ArgumentTypeError("expected NAME=VALUE")
    name, env_value = value.split("=", 1)
    if not name:
        raise argparse.ArgumentTypeError("environment variable name is empty")
    return name, env_value


def strip_terminal(text: str) -> str:
    text = re.sub(r"\x1b\[[0-9;?<>]*[A-Za-z~]", "", text)
    text = re.sub(r"\x1b\][^\x07]*(?:\x07|\x1b\\)", "", text)
    text = re.sub(r"\x1b[()][A-Z0-9]|\x1b[=>78]", "", text)
    return text


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--bin", required=True, help="CLI executable to run")
parser.add_argument("--cwd", required=True, help="scratch working directory")
parser.add_argument("--cols", type=int, required=True)
parser.add_argument("--rows", type=int, required=True)
parser.add_argument("--env", action="append", default=[], type=parse_env, metavar="NAME=VALUE")
parser.add_argument("--keys", required=True, help="characters to type after the CLI is ready")
parser.add_argument("--key-delay-ms", type=int, default=150)
parser.add_argument("--settle", type=float, default=6.0)
parser.add_argument("--out", required=True, help="JSONL recording path")
parser.add_argument(
    "--resize-after-ms", type=int, help="resize this many milliseconds after typing starts"
)
parser.add_argument("--resize-to", type=parse_size, metavar="COLSxROWS")
parser.add_argument("--extra", default="", help="extra CLI arguments, parsed like a shell command")
args = parser.parse_args()

if args.cols < 1 or args.rows < 1:
    parser.error("--cols and --rows must be positive")
if args.key_delay_ms < 0 or args.settle < 0:
    parser.error("delays must be non-negative")
if (args.resize_after_ms is None) != (args.resize_to is None):
    parser.error("--resize-after-ms and --resize-to must be supplied together")
if args.resize_after_ms is not None and args.resize_after_ms < 0:
    parser.error("--resize-after-ms must be non-negative")

binary = os.path.abspath(os.path.expanduser(args.bin))
if not os.path.isfile(binary) or not os.access(binary, os.X_OK):
    parser.error(f"--bin is not an executable file: {binary}")

scratch = Path(args.cwd).expanduser().resolve()
scratch.mkdir(parents=True, exist_ok=True)
out_path = Path(args.out).expanduser().resolve()
out_path.parent.mkdir(parents=True, exist_ok=True)

start = time.monotonic()
events: list[dict[str, object]] = []
raw_output = bytearray()
lock = threading.Lock()
last_rx = [start]
reading = [True]


def elapsed_ms() -> int:
    return round((time.monotonic() - start) * 1000)


def append_event(kind: str, **fields: object) -> None:
    with lock:
        events.append({"t_ms": elapsed_ms(), "kind": kind, **fields})


pid, fd = pty.fork()
if pid == 0:
    os.chdir(scratch)
    env = {
        key: value
        for key, value in os.environ.items()
        if not (
            key.startswith("CLAUDE_CODE")
            or key in {"CLAUDECODE", "TERM_PROGRAM", "TERM_PROGRAM_VERSION"}
        )
    }
    env.update(
        {
            "TERM": "xterm-256color",
            "COLORTERM": "truecolor",
            "LANG": "en_US.UTF-8",
        }
    )
    env.update(dict(args.env))
    argv = [binary, *shlex.split(args.extra)]
    os.execve(binary, argv, env)


def record_resize(cols: int, rows: int) -> None:
    # Hold the event lock across TIOCSWINSZ so SIGWINCH output cannot be
    # recorded before the resize event that caused it.
    with lock:
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        events.append({"t_ms": elapsed_ms(), "kind": "resize", "cols": cols, "rows": rows})


record_resize(args.cols, args.rows)


def reader() -> None:
    while reading[0]:
        ready, _, _ = select.select([fd], [], [], 0.2)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        with lock:
            if not reading[0]:
                break
            raw_output.extend(chunk)
            events.append(
                {
                    "t_ms": elapsed_ms(),
                    "kind": "out",
                    "b64": base64.b64encode(chunk).decode("ascii"),
                }
            )
            last_rx[0] = time.monotonic()


reader_thread = threading.Thread(target=reader, daemon=True)
reader_thread.start()


def output_text() -> str:
    with lock:
        return raw_output.decode("utf-8", "replace")


def write_all(data: bytes) -> None:
    view = memoryview(data)
    while view:
        written = os.write(fd, view)
        view = view[written:]


def quiet_for(seconds: float) -> bool:
    with lock:
        return time.monotonic() - last_rx[0] >= seconds


def stop_child() -> None:
    # Freeze the recording before termination: exit cleanup would erase the TUI
    # frame this harness is meant to replay.
    reading[0] = False
    try:
        os.kill(pid, signal.SIGTERM)
        time.sleep(0.25)
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    reader_thread.join(timeout=1.0)
    try:
        os.close(fd)
    except OSError:
        pass


try:
    ready_re = re.compile(r'Try\s*"|shortcuts|mode\s*on', re.IGNORECASE)
    deadline = time.monotonic() + 60
    trusted = False
    while time.monotonic() < deadline:
        time.sleep(0.25)
        plain = strip_terminal(output_text())
        if not trusted and re.search(r"trust\s*this\s*folder", plain, re.IGNORECASE):
            # Claude starts on "No". Its incremental redraw moves only the
            # cursor, so the concatenated byte stream never contains a fresh
            # "Yes" label to parse after Down; drive the deterministic choice
            # exactly once instead.
            write_all(b"\x1b[B")
            time.sleep(0.2)
            write_all(b"\r")
            trusted = True
            time.sleep(1.5)
            continue
        if ready_re.search(plain) and quiet_for(1.5):
            break
    else:
        print("NOT READY after 60s; stripped output tail:", file=sys.stderr)
        print(strip_terminal(output_text())[-1500:], file=sys.stderr)
        sys.exit(2)

    time.sleep(1.0)
    typing_started = time.monotonic()

    def resize_later() -> None:
        assert args.resize_after_ms is not None and args.resize_to is not None
        deadline_at = typing_started + args.resize_after_ms / 1000
        delay = deadline_at - time.monotonic()
        if delay > 0:
            time.sleep(delay)
        if not reading[0]:
            return
        cols, rows = args.resize_to
        record_resize(cols, rows)

    resize_thread = None
    if args.resize_after_ms is not None:
        resize_thread = threading.Thread(target=resize_later, daemon=True)
        resize_thread.start()

    for character in args.keys:
        encoded = character.encode("utf-8")
        append_event("key", b64=base64.b64encode(encoded).decode("ascii"))
        write_all(encoded)
        time.sleep(args.key_delay_ms / 1000)

    time.sleep(args.settle)
    if resize_thread is not None:
        resize_thread.join(timeout=0.5)
finally:
    stop_child()

with out_path.open("w", encoding="utf-8") as stream:
    for event in events:
        stream.write(json.dumps(event, separators=(",", ":")) + "\n")

raw_path = Path(str(out_path) + ".txt")
raw_path.write_bytes(raw_output)
print(
    f"recorded {len(events)} events / {len(raw_output)} output bytes "
    f"to {out_path} (raw: {raw_path})"
)
