#!/usr/bin/env python3
"""A full-screen TUI stand-in: on start and on every SIGWINCH it clears the
screen and paints every row with its row number, the size it sees and a
repaint serial, the way an agent's TUI repaints itself on resize."""
import os
import signal
import sys
import time

serial = 0
pending = True


def on_winch(*_):
    global pending
    pending = True


def paint():
    global serial
    serial += 1
    cols, rows = os.get_terminal_size(sys.stdout.fileno())
    out = ["\x1b[?25l\x1b[H\x1b[2J"]
    for y in range(rows):
        head = f"R{y:02} {cols}x{rows} #{serial} "
        fill = "".join(chr(ord("a") + (y + i) % 26) for i in range(max(0, cols - len(head))))
        out.append(f"\x1b[{y + 1};1H{(head + fill)[:cols]}")
    out.append(f"\x1b[{rows};{cols}H")
    sys.stdout.write("".join(out))
    sys.stdout.flush()


signal.signal(signal.SIGWINCH, on_winch)
sys.stdout.write("\x1b[?1049h")
while True:
    if pending:
        pending = False
        paint()
    time.sleep(0.005)
