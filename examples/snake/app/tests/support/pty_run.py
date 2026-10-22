"""Exercise the real Snake executable through a controlling Unix terminal."""

import errno
import fcntl
import os
import re
import select
import subprocess
import sys
import termios
import time


def supervise(status_fd, release_fd, command):
    """Keep the terminal session alive until the parent checks restored flags.

    macOS revokes a controlling terminal when its session leader exits.
    This supervisor preserves the terminal after the application exits.
    """
    child = subprocess.Popen(command)
    reported = False
    try:
        while not select.select([release_fd], [], [], 0.05)[0]:
            if child.poll() is not None and not reported:
                os.write(status_fd, str(child.returncode).encode())
                reported = True
    finally:
        if child.poll() is None:
            child.kill()
        child.wait(timeout=5)


def exercise(command):
    """Catch missing arrow delivery, premature shutdown, and terminal leaks."""
    master, slave = os.openpty()
    status_read, status_write = os.pipe()
    release_read, release_write = os.pipe()
    original = termios.tcgetattr(slave)
    output = bytearray()
    process = None
    exit_code = None

    def terminal_session():
        """Give crossterm a controlling terminal, including /dev/tty."""
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    def receive_until(predicate, description, seconds=15):
        """Read terminal events until the condition or its deadline occurs."""
        nonlocal exit_code
        deadline = time.monotonic() + seconds
        while not predicate(bytes(output)):
            remaining = deadline - time.monotonic()
            assert remaining > 0, f"timeout: {description}\n{output.decode(errors='replace')}"
            readers = [master] + ([status_read] if exit_code is None else [])
            ready = select.select(readers, [], [], min(remaining, 0.1))[0]
            if master in ready:
                try:
                    chunk = os.read(master, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    chunk = b""
                assert chunk, f"terminal ended before {description}\n{output.decode(errors='replace')}"
                output.extend(chunk)
            if status_read in ready:
                exit_code = int(os.read(status_read, 32))
            if exit_code is not None and not ready and not predicate(bytes(output)):
                raise AssertionError(
                    f"exit {exit_code} before {description}\n" + output.decode(errors="replace")
                )

    def head_positions(data):
        """Find complete 16-row game frames after ANSI color removal."""
        text = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", data)
        rows = re.findall(rb"\|([^\r\n]{32})\|", text)
        positions = []
        for offset in range(0, len(rows) - 15, 16):
            frame = rows[offset:offset + 16]
            positions.extend(
                (row, cells.index(b"@") // 2)
                for row, cells in enumerate(frame)
                if b"@" in cells
            )
        return positions

    try:
        process = subprocess.Popen(
            [sys.executable, __file__, "--session", str(status_write), str(release_read), *command],
            stdin=slave, stdout=slave, stderr=slave,
            preexec_fn=terminal_session, pass_fds=(status_write, release_read),
            env={**os.environ, "TERM": "xterm-256color"},
        )
        receive_until(lambda data: (8, 8) in head_positions(data), "initial game frame")
        raw = termios.tcgetattr(slave)
        assert not raw[3] & (termios.ECHO | termios.ICANON | termios.ISIG), "terminal is not raw"
        os.write(master, b"\x1b[A")
        receive_until(lambda data: (7, 8) in head_positions(data), "Up-arrow turn")
        os.write(master, b"\x03")
        receive_until(lambda data: b"Game over! Your score was:" in data, "shutdown score")
        if exit_code is None:
            assert select.select([status_read], [], [], 5)[0], "application did not terminate"
            exit_code = int(os.read(status_read, 32))
        assert exit_code == 0, output.decode(errors="replace")
        assert termios.tcgetattr(slave) == original, "terminal attributes were not restored"
        print("PTY PASS: game ready; Up-arrow turn; Ctrl-C exit 0; score; terminal restored")
    finally:
        os.write(release_write, b"done")
        if process is not None:
            process.wait(timeout=5)
        for descriptor in [master, slave, status_read, status_write, release_read, release_write]:
            os.close(descriptor)


if __name__ == "__main__":
    if sys.argv[1] == "--session":
        supervise(int(sys.argv[2]), int(sys.argv[3]), sys.argv[4:])
    else:
        exercise(sys.argv[1:])
