#!/usr/bin/env python3
"""Reproduce Conclave's message.inject write pattern against a real CLI TUI in a PTY.

Mirrors src-tauri/src/engine/commands/message.rs::inject exactly:
  one write of the tagged body, then bare "\r" at +40ms, +120ms, +300ms.

Modes:
  plain      — body written as-is (current Conclave behaviour)
  bracketed  — body wrapped in ESC[200~ … ESC[201~ (candidate fix)

Reports what the receiver actually SUBMITTED by reading the CLI's own transcript.

Usage (claude):
  python3 scripts/pty-inject-repro.py --bin ~/.local/share/claude/versions/<v> \
      --cwd /tmp/some-scratch-dir --size 2206 --mode bracketed --extra "--model haiku"
  → prints RESULT: INTACT / TRUNCATED with the received byte count.

Caveats: drives claude's first-run trust dialog for you; it does NOT drive
codex's dialogs (a codex self-update prompt will be answered by the harness's
readiness keystrokes — run codex once by hand in the scratch dir first). Each
run costs one small model turn in the scratch dir.
"""
import argparse, fcntl, glob, json, os, pty, re, select, signal, struct, sys, termios, threading, time, uuid

ap = argparse.ArgumentParser()
ap.add_argument("--bin", required=True)
ap.add_argument("--cwd", required=True)
ap.add_argument("--size", type=int, required=True, help="total body bytes (incl. [from …] tag)")
ap.add_argument("--mode", choices=["plain", "bracketed"], default="plain")
ap.add_argument("--cli", choices=["claude", "codex"], default="claude")
ap.add_argument("--settle", type=float, default=10.0, help="seconds to wait after the CRs")
ap.add_argument("--extra", default="", help="extra CLI args (space separated)")
ap.add_argument("--dump", action="store_true", help="print stripped terminal output at the end")
args = ap.parse_args()

os.makedirs(args.cwd, exist_ok=True)
t0 = time.time()

pid, fd = pty.fork()
if pid == 0:
    os.chdir(args.cwd)
    # scrub the parent Claude Code session's markers: a child session inherits
    # CLAUDE_CODE_CHILD_SESSION and then refuses to persist its transcript
    env = {k: v for k, v in os.environ.items() if not (k.startswith("CLAUDE_CODE") or k == "CLAUDECODE")}
    env.update({"TERM": "xterm-256color", "COLORTERM": "truecolor", "LANG": "en_US.UTF-8",
                "CLAUDE_CODE_FORCE_SESSION_PERSISTENCE": "1"})
    argv = [args.bin] + (args.extra.split() if args.extra else [])
    os.execve(args.bin, argv, env)

# window size like a real pane
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 200, 0, 0))

out = bytearray()
lock = threading.Lock()
last_rx = [time.time()]
alive = [True]

def reader():
    while alive[0]:
        r, _, _ = select.select([fd], [], [], 0.2)
        if not r:
            continue
        try:
            b = os.read(fd, 65536)
        except OSError:
            break
        if not b:
            break
        with lock:
            out.extend(b)
            last_rx[0] = time.time()

th = threading.Thread(target=reader, daemon=True)
th.start()

def text():
    with lock:
        return out.decode("utf-8", "replace")

def write_all(b: bytes):
    # master writes BLOCK when the slave's input queue is full (TTYHOG); loop on partials
    view = memoryview(b)
    while len(view):
        n = os.write(fd, view)
        view = view[n:]

def quiet_for(sec):
    return time.time() - last_rx[0] >= sec

# ---- wait for readiness (and drive the trust dialog if one appears) ----
ready_re = re.compile(r'Try\s*"|shortcuts|mode\s*on' if args.cli == "claude" else r"›|Ask Codex|for commands", re.I)
deadline = time.time() + 60
trusted = False
last_nudge = 0.0
def stripped():
    return re.sub(r"\x1b\[[0-9;?<>]*[A-Za-z~]|\x1b\][^\x07]*\x07|\x1b[()][A-Z0-9]|\x1b[=>78]", "", text())
while time.time() < deadline:
    time.sleep(0.25)
    plain = stripped()
    if args.cli == "claude" and not trusted and re.search(r"trust\s*this\s*folder", plain, re.I):
        # find the CURRENT selection: last "❯" followed by No/Yes
        sel = re.findall(r"❯\s*(No|Yes)", plain)
        cur = sel[-1] if sel else None
        if cur == "Yes":
            write_all(b"\r")
            trusted = True
            time.sleep(1.5)
        elif time.time() - last_nudge > 0.6:
            write_all(b"\x1b[B")
            last_nudge = time.time()
        continue
    if args.cli == "codex" and re.search(r"Ask Codex", plain):
        time.sleep(6.0)  # let MCP servers finish starting; the spinner never goes quiet
        break
    if ready_re.search(plain) and quiet_for(1.5):
        break
else:
    print("NOT READY after 60s; tail of output:\n", stripped()[-1500:])
    os.kill(pid, signal.SIGKILL)
    sys.exit(2)

time.sleep(1.0)

# ---- build the body: exact byte size, unmistakable head/tail markers, no newlines ----
sender_id = str(uuid.uuid4())
head = f"[from Harness · {sender_id}] HEAD-MARKER size={args.size} mode={args.mode} :: "
tail = " :: TAIL-MARKER. Reply with the single word OK."
filler_words = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel"]
body = head
i = 0
target = args.size - len(tail.encode())
while len(body.encode()) < target:
    body += filler_words[i % len(filler_words)] + " "
    i += 1
body = body.encode()[:target].decode("utf-8", "ignore")
body = body + tail
body_b = body.encode()
assert abs(len(body_b) - args.size) <= 4, (len(body_b), args.size)

payload = body_b
if args.mode == "bracketed":
    payload = b"\x1b[200~" + body_b + b"\x1b[201~"

with lock:
    out_len_before = len(out)

ts_write = time.time()
def dump_term(label):
    plain_out = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07|\x1b[()][A-Z0-9]|\x1b[=>]", "", text())
    plain_out = re.sub(r"\n\s*\n+", "\n", plain_out)
    print(f"---- terminal {label} (stripped, tail) ----")
    print(plain_out[-3000:])
    print("---- end terminal ----")
try:
    write_all(payload)
    for delay_ms in (40, 120, 300):
        time.sleep(delay_ms / 1000)
        write_all(b"\r")
except OSError as e:
    print("WRITE FAILED:", e)
    dump_term("at write failure")
    sys.exit(3)

time.sleep(args.settle)

# ---- graceful-ish exit ----
try:
    os.kill(pid, signal.SIGTERM)
    time.sleep(0.5)
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError:
    pass
alive[0] = False

# ---- read back what was SUBMITTED from the CLI's own transcript ----
def claude_transcript_user_msgs():
    mangled = re.sub(r"[^A-Za-z0-9]", "-", args.cwd)
    proj = os.path.expanduser(f"~/.claude/projects/{mangled}")
    files = [p for p in glob.glob(os.path.join(proj, "*.jsonl")) if os.path.getmtime(p) >= t0 - 1]
    msgs = []
    for p in files:
        for line in open(p, encoding="utf-8"):
            try:
                o = json.loads(line)
            except Exception:
                continue
            if o.get("type") != "user":
                continue
            c = o.get("message", {}).get("content")
            if isinstance(c, list):
                tx = "\n".join(b.get("text", "") for b in c if isinstance(b, dict) and b.get("type") == "text")
            else:
                tx = str(c)
            if tx:
                msgs.append(tx)
    return files, msgs

def codex_transcript_user_msgs():
    root = os.path.expanduser("~/.codex/sessions")
    files = [p for p in glob.glob(os.path.join(root, "**", "*.jsonl"), recursive=True) if os.path.getmtime(p) >= t0 - 1]
    msgs = []
    for p in files:
        for line in open(p, encoding="utf-8"):
            try:
                o = json.loads(line)
            except Exception:
                continue
            pl = o.get("payload", {}) if isinstance(o, dict) else {}
            if pl.get("type") == "user_message" and isinstance(pl.get("message"), str):
                msgs.append(pl["message"])
            elif pl.get("type") == "message" and pl.get("role") == "user":
                cont = pl.get("content", [])
                tx = "\n".join(b.get("text", "") for b in cont if isinstance(b, dict))
                if tx:
                    msgs.append(tx)
    return files, msgs

if args.dump:
    dump_term("at end")
files, msgs = (claude_transcript_user_msgs() if args.cli == "claude" else codex_transcript_user_msgs())
print(f"=== {args.cli} {os.path.basename(args.bin)} mode={args.mode} sizeB={len(body_b)} ===")
print(f"transcript files: {[os.path.basename(f) for f in files]}")
if not msgs:
    print("RESULT: NO user message recorded (nothing submitted)")
for m in msgs:
    mb = len(m.encode())
    has_head = "HEAD-MARKER" in m
    has_tail = "TAIL-MARKER" in m
    verdict = "INTACT" if (has_head and has_tail and mb >= len(body_b) - 2) else "TRUNCATED/ALTERED"
    print(f"RESULT: {verdict} recvB={mb} head={has_head} tail={has_tail} first60={m[:60]!r} last40={m[-40:]!r}")
