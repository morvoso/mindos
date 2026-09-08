#!/usr/bin/env python3
"""Disposable guest-only keyboard for media QA. Requires root and /dev/uinput."""
import fcntl
import os
import signal
import socket
import struct
import time

path = '/run/mindos-qa-media-keys.sock'

def stop(*_):
    raise SystemExit(0)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGALRM, stop)
signal.alarm(180)
fd = os.open('/dev/uinput', os.O_WRONLY | os.O_NONBLOCK)
server = socket.socket(socket.AF_UNIX)
try:
    fcntl.ioctl(fd, 0x40045564, 1)  # EV_KEY
    for key in range(1, 249):
        fcntl.ioctl(fd, 0x40045565, key)
    fcntl.ioctl(fd, 0x405c5503, struct.pack('HHHH80sI', 3, 1, 1, 1, b'MindOS QA media keys', 0))
    fcntl.ioctl(fd, 0x5501)
    server.bind(path)
    os.chmod(path, 0o600)
    server.listen(1)
    while True:
        connection, _ = server.accept()
        with connection:
            connection.settimeout(2)
            key, down = map(int, connection.recv(64).decode().split())
            assert 1 <= key < 249 and down in (0, 1)
            os.write(fd, struct.pack('llHHi', 0, 0, 1, key, down))
            os.write(fd, struct.pack('llHHi', 0, 0, 0, 0, 0))
            connection.sendall(b'OK')
finally:
    fcntl.ioctl(fd, 0x5502)
    os.close(fd)
    server.close()
    if os.path.exists(path):
        os.unlink(path)
