#!/usr/bin/env python3
"""A full-screen TUI stand-in that paints the way an agent's terminal UI
does: it probes the terminal at start (XTVERSION, kitty keyboard, DA1,
DECRQM 2026, CSI 16t), takes the alternate screen, and on start and on
every SIGWINCH repaints inside a synchronized-output hold (mode 2026):
clear, home, then every row with relative moves, the size it sees and a
repaint serial, cursor parked with absolute moves at the end."""
import os
import select
import signal
import sys
import time

serial = 0
pending = True


def on_winch(*_):
    global pending
    pending = True


def write(s):
    sys.stdout.write(s)
    sys.stdout.flush()


def drain():
    # Replies to the probes come back as input; read and drop them.
    while select.select([sys.stdin], [], [], 0)[0]:
        if not os.read(sys.stdin.fileno(), 4096):
            break


def paint():
    global serial
    serial += 1
    cols, rows = os.get_terminal_size(sys.stdout.fileno())
    out = ["\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[?2026h\x1b[?25l\x1b[2J\x1b[H"]
    for y in range(rows):
        head = f"R{y:02} {cols}x{rows} #{serial} "
        fill = "".join(chr(ord("a") + (y + i) % 26) for i in range(max(0, cols - len(head))))
        if y > 0:
            out.append("\r\x1b[1B")
        out.append((head + fill)[:cols])
    out.append(f"\x1b[{rows};1H\x1b[{max(1, rows - 3)};3H\x1b[?25h\x1b[?2026l")
    write("".join(out))


signal.signal(signal.SIGWINCH, on_winch)
write("\x1b7\x1b[r\x1b8\x1b[?25h\x1b[?25l\x1b[?2004h\x1b[?2031h\x1b[?1004h\x1b[<u\x1b[>5u\x1b[>4;2m\x1b[>0q\x1b[?u\x1b]7501;?\x1b\\\x1b[c")
time.sleep(0.1)
drain()
write("\x1b[?2026$p\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[16t\x1b[?1016$p\x1b[c")
time.sleep(0.1)
drain()
write("\x1b]0;stand-in\x07\x1b[?1049h\x1b[2J\x1b[H\x1b[<u\x1b[>5u\x1b[>4;2m")
while True:
    if pending:
        pending = False
        paint()
    drain()
    time.sleep(0.005)
