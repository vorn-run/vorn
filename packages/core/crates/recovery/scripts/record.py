#!/usr/bin/env python3
"""Record a program's terminal output as a VREC1 transcript (see src/transcript.rs).

Runs the program on a fresh PTY with a clean environment (TERM, LANG, PATH and
an empty HOME, nothing inherited), types a scripted sequence of keys and
resizes into it, and writes every read of the PTY as one Data record and every
resize as a Resize record, each with the time since the one before.

    python3 scripts/record.py vim transcripts/vim.rec
    python3 scripts/record.py htop transcripts/htop.rec
    python3 scripts/record.py claude transcripts/claude.rec

Linux or macOS. Re-recording changes the bytes; the tests that use the
transcripts compare a run against another run of the same file, never against
stored output, so any recording works. Before committing one, check it holds
nothing about the machine or an account (see the crate's README).
"""

import fcntl
import os
import pty
import select
import shutil
import signal
import struct
import subprocess
import sys
import termios
import time

WORK = "/tmp/vorn-transcript"
COLS, ROWS = 80, 24


def clean_env(home):
    return {
        "TERM": "xterm-256color",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "HOME": home,
        "USER": "user",
        "LOGNAME": "user",
        "SHELL": "/bin/sh",
        "PATH": "/usr/local/bin:/usr/bin:/bin",
    }


def fresh_dir(path):
    shutil.rmtree(path, ignore_errors=True)
    os.makedirs(path)
    return path


def scenario_vim():
    home = fresh_dir(WORK + "/home")
    work = fresh_dir(WORK + "/work")
    with open(os.path.join(work, "notes.txt"), "w") as f:
        for i in range(60):
            f.write(f"line {i:02d}: the quick brown fox jumps over the lazy dog\n")
    steps = [
        (1.0, b"5j"), (0.3, b"w"), (0.3, b"iinserted text \x1b"), (0.4, b":split\r"),
        (0.5, ("resize", 100, 30)), (0.6, b"\x17j"), (0.3, b"Go\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e and emoji \xf0\x9f\x98\x80\x1b"),
        (0.4, b"gg"), (0.3, ("resize", 70, 20)), (0.6, b"/fox\r"), (0.3, b"n"),
        (0.3, b"\x06"), (0.3, b"\x06"), (0.3, b"\x02"), (0.3, b"dd"), (0.3, b"u"), (0.3, b":set number\r"),
        (0.3, b"10G"), (0.3, b"cwchanged\x1b"), (0.3, b"\x17c"), (0.3, b":vsplit\r"), (0.4, ("resize", 90, 26)),
        (0.4, b"\x17l"), (0.3, b"G"), (0.4, b":qa!\r"),
    ]
    return ["vim", "notes.txt"], clean_env(home), work, steps, 15.0


def scenario_htop():
    home = fresh_dir(WORK + "/home")
    work = fresh_dir(WORK + "/work")
    env = clean_env(home)
    # Only processes started here are shown, so nothing else on the machine
    # ends up in the transcript.
    sleepers = [subprocess.Popen(["sleep", "300"], env=env, cwd=work) for _ in range(3)]
    pids = ",".join(str(p.pid) for p in sleepers)
    steps = [
        (1.5, b"\x1b[B"), (0.5, ("resize", 100, 30)), (1.5, b"t"), (1.0, ("resize", 72, 20)),
        (1.0, b"\x1b[A"), (1.0, b"t"), (1.0, ("resize", 80, 24)), (1.5, b"q"),
    ]
    return ["htop", "-d", "5", "-p", pids], env, work, steps, 12.0, sleepers


def scenario_claude():
    home = fresh_dir(WORK + "/home")
    work = fresh_dir(WORK + "/work")
    env = clean_env(home)
    env["PATH"] = "/opt/node22/bin:" + env["PATH"]
    # No credentials and no traffic that is not needed to draw the first screens.
    env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"] = "1"
    env["DISABLE_AUTOUPDATER"] = "1"
    steps = [(4.0, b"\x1b[B"), (1.0, ("resize", 100, 30)), (1.5, b"\x03"), (0.5, b"\x03")]
    return ["claude"], env, work, steps, 10.0


SCENARIOS = {"vim": scenario_vim, "htop": scenario_htop, "claude": scenario_claude}


def set_size(fd, cols, rows):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def record(argv, env, cwd, steps, limit, out):
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(cwd)
        os.execvpe(argv[0], argv, env)
    set_size(fd, COLS, ROWS)
    records = []
    last = time.monotonic()
    start = last
    due = start
    queue = list(steps)

    def stamp():
        nonlocal last
        now = time.monotonic()
        delta = int((now - last) * 1e6)
        last = now
        return min(delta, 0xFFFFFFFF)

    while time.monotonic() - start < limit:
        if queue and time.monotonic() >= due + queue[0][0]:
            delay, action = queue.pop(0)
            due += delay
            if isinstance(action, tuple):
                _, cols, rows = action
                set_size(fd, cols, rows)
                records.append(b"R" + struct.pack("<IHH", stamp(), cols, rows))
            else:
                os.write(fd, action)
        ready, _, _ = select.select([fd], [], [], 0.02)
        if ready:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            records.append(b"D" + struct.pack("<II", stamp(), len(data)) + data)
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    os.waitpid(pid, 0)
    with open(out, "wb") as f:
        f.write(b"VREC1\n" + struct.pack("<HH", COLS, ROWS) + b"".join(records))
    print(f"{out}: {len(records)} records, {os.path.getsize(out)} bytes")


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in SCENARIOS:
        sys.exit(f"usage: record.py {{{'|'.join(SCENARIOS)}}} OUT.rec")
    scenario = SCENARIOS[sys.argv[1]]()
    argv, env, cwd, steps, limit = scenario[:5]
    try:
        record(argv, env, cwd, steps, limit, sys.argv[2])
    finally:
        for p in scenario[5] if len(scenario) > 5 else []:
            p.kill()
        shutil.rmtree(WORK, ignore_errors=True)


if __name__ == "__main__":
    main()
