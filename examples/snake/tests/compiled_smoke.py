"""Build and exercise the generated launchers in a real Unix pseudo-terminal.

Run from any directory: python3 examples/snake/tests/compiled_smoke.py
"""

import json
import os
from pathlib import Path
import pty
import re
import select
import subprocess
import termios
import time
import unittest


EXAMPLE = Path(__file__).resolve().parents[1]
ROOT = EXAMPLE.parents[1]
ANSI = re.compile(rb"\x1b\[[0-9;?]*[A-Za-z]")


class Terminal:
    def __init__(self, executable):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.process = subprocess.Popen(
            executable if isinstance(executable, list) else [executable],
            stdin=self.slave, stdout=self.slave, stderr=self.slave, cwd=ROOT,
            env={**os.environ, "RUST_LOG": "error"},
        )
        self.output = b""

    def wait_for(self, predicate, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            if select.select([self.master], [], [], 0.05)[0]:
                self.output += os.read(self.master, 65536)
        raise AssertionError(f"Terminal condition timed out: {self.output[-2000:]!r}")

    def send(self, data):
        os.write(self.master, data)

    def close(self):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=5)
        os.close(self.master)
        os.close(self.slave)


class CompiledSnake(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.executables = {}
        for deployment in ("snake", "keyboard"):
            workspace = EXAMPLE if deployment == "snake" else EXAMPLE / "keyboard"
            result = subprocess.run(
                ["cargo", "run", "--quiet", "-p", "cargo-boomerang", "--",
                 "boomerang", "--workspace", str(workspace), "build",
                 "--deployment", deployment],
                cwd=ROOT, text=True, stdout=subprocess.PIPE, check=True,
            )
            bundle = Path(result.stdout.strip())
            document = json.loads(bundle.read_text())
            artifacts = document["artifacts"]
            cls.executables[deployment] = bundle.parent / artifacts[0]["path"]

    def launch(self, deployment):
        terminal = Terminal(self.executables[deployment])
        self.addCleanup(terminal.close)
        terminal.wait_for(lambda: not (
            termios.tcgetattr(terminal.slave)[3] & termios.ICANON
        ))
        return terminal

    def stop(self, terminal):
        terminal.send(b"\x03")
        self.assertEqual(terminal.process.wait(timeout=5), 0)
        self.assertEqual(termios.tcgetattr(terminal.slave), terminal.original)

    def test_keyboard_delivers_all_arrows_and_restores_terminal(self):
        terminal = self.launch("keyboard")
        for sequence, arrow in ((b"\x1b[A", "↑"), (b"\x1b[B", "↓"),
                                (b"\x1b[C", "→"), (b"\x1b[D", "←")):
            terminal.send(sequence)
            terminal.wait_for(lambda: arrow.encode() in terminal.output)
        self.stop(terminal)

    def test_cli_run_keeps_keyboard_input_interactive(self):
        terminal = Terminal([
            "cargo", "run", "--quiet", "-p", "cargo-boomerang", "--", "boomerang",
            "--workspace", str(EXAMPLE / "keyboard"), "run", "--deployment", "keyboard",
        ])
        self.addCleanup(terminal.close)
        terminal.wait_for(lambda: not (
            termios.tcgetattr(terminal.slave)[3] & termios.ICANON
        ), timeout=30)
        terminal.send(b"\x1b[A")
        terminal.wait_for(lambda: "↑".encode() in terminal.output)
        self.stop(terminal)

    def test_snake_renders_and_turns_on_the_refresh_clock(self):
        terminal = self.launch("snake")

        def grids():
            frames = ANSI.sub(b"", terminal.output).split(b"+" + b"~" * 32 + b"+")
            result = []
            for frame in frames:
                rows = [row for row in frame.replace(b"\r", b"").split(b"\n")
                        if row.startswith(b"|")]
                if len(rows) == 16:
                    result.append(rows)
            return result

        def heads():
            return [next((r, (row.index(b"@") - 1) // 2)
                         for r, row in enumerate(rows) if b"@" in row)
                    for rows in grids()]

        terminal.wait_for(lambda: len(heads()) >= 1)
        started = time.monotonic()
        self.assertEqual(heads()[0], (8, 8))
        terminal.send(b"\x1b[A")
        terminal.wait_for(lambda: len(heads()) >= 2)
        self.assertEqual(heads()[1], (7, 8))
        terminal.send(b"\x1b[B")  # An immediate reversal must be ignored.
        terminal.wait_for(lambda: len(heads()) >= 3)
        self.assertEqual(heads()[2], (6, 8))

        # Avoid food and the body while waiting for the independent food clock.
        # Reading actual cells keeps this deterministic despite random food placement.
        direction = (-1, 0)
        keys = {(-1, 0): b"\x1b[A", (1, 0): b"\x1b[B",
                (0, 1): b"\x1b[C", (0, -1): b"\x1b[D"}
        while time.monotonic() - started < 6:
            grid = grids()[-1]
            food = sum(row.count(b"x") for row in grid)
            if time.monotonic() - started < 4:
                self.assertLess(food, 2)
            if food == 2:
                break
            row, col = heads()[-1]
            for candidate in [direction, *keys]:
                if candidate == (-direction[0], -direction[1]):
                    continue
                next_row, next_col = (row + candidate[0]) % 16, (col + candidate[1]) % 16
                if grid[next_row][1 + 2 * next_col] == ord(" "):
                    direction = candidate
                    break
            else:
                self.fail("No free step while checking the food clock")
            count = len(grids())
            terminal.send(keys[direction])
            terminal.wait_for(lambda: len(grids()) > count)
        self.assertEqual(sum(row.count(b"x") for row in grids()[-1]), 2)
        self.stop(terminal)
        terminal.wait_for(lambda: b"Game over! Your score was:" in terminal.output)


if __name__ == "__main__":
    unittest.main()
