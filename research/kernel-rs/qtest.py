#!/usr/bin/env python3
"""Boot MindOS in QEMU headlessly and check the serial log for markers.

usage: qtest.py [--timeout N] [--expect MARKER ...] [--screenshot file.png]
"""
import argparse, os, socket, subprocess, sys, time, zlib, struct

def ppm_to_png(ppm_path, png_path):
    data = open(ppm_path, 'rb').read()
    # parse P6 header
    parts = []
    pos = 0
    while len(parts) < 4:
        while data[pos:pos+1].isspace():
            pos += 1
        if data[pos:pos+1] == b'#':
            while data[pos:pos+1] not in (b'\n', b''):
                pos += 1
            continue
        start = pos
        while not data[pos:pos+1].isspace():
            pos += 1
        parts.append(data[start:pos])
    pos += 1
    w, h = int(parts[1]), int(parts[2])
    pix = data[pos:pos + w * h * 3]
    raw = b''.join(b'\x00' + pix[y * w * 3:(y + 1) * w * 3] for y in range(h))
    def chunk(t, d):
        c = struct.pack('>I', len(d)) + t + d
        return c + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(raw, 6)) + chunk(b'IEND', b'')
    open(png_path, 'wb').write(png)

def monitor_cmd(sock_path, cmd):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(5)
    s.connect(sock_path)
    time.sleep(0.2)
    try:
        s.recv(4096)
    except Exception:
        pass
    s.sendall((cmd + "\n").encode())
    time.sleep(0.5)
    try:
        out = s.recv(65536)
    except Exception:
        out = b''
    s.close()
    return out.decode(errors='replace')

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--timeout', type=float, default=40)
    ap.add_argument('--expect', action='append', default=None)
    ap.add_argument('--screenshot', default=None)
    ap.add_argument('--keys', default=None, help='sendkey sequence after boot markers, e.g. "h e l p ret"')
    ap.add_argument('--serial-in', default=None, help='text to type on the serial console after expected markers')
    ap.add_argument('--after-expect', action='append', default=[], help='markers expected after keys/serial input')
    ap.add_argument('--mem', default='4G')
    ap.add_argument('--cpus', default='4')
    ap.add_argument('--no-kvm', action='store_true')
    ap.add_argument('--disk', default='build/mindos.img')
    args = ap.parse_args()
    expect = args.expect or ['bring-up complete']
    os.makedirs('build', exist_ok=True)
    serial_log = 'build/serial.log'
    mon = '/tmp/mindos-qtest-mon.sock'
    serial_sock = '/tmp/mindos-qtest-serial.sock'
    for p in (mon, serial_sock):
        try:
            os.unlink(p)
        except FileNotFoundError:
            pass
    cmd = ['qemu-system-x86_64', '-machine', 'q35', '-m', args.mem, '-smp', args.cpus,
           '-drive', f'file={args.disk},format=raw,if=none,id=hd0', '-device', 'virtio-blk-pci,drive=hd0',
           '-no-reboot', '-no-shutdown', '-display', 'none',
           '-chardev', f'socket,id=ser0,path={serial_sock},server=on,wait=off,logfile={serial_log}',
           '-serial', 'chardev:ser0',
           '-monitor', f'unix:{mon},server,nowait',
           '-debugcon', 'file:build/debugcon.log', '-global', 'isa-debugcon.iobase=0xe9']
    if os.path.exists('build/data.img'):
        cmd += ['-drive', 'file=build/data.img,format=raw,if=none,id=hd1', '-device', 'virtio-blk-pci,drive=hd1']
    if not args.no_kvm and os.path.exists('/dev/kvm'):
        cmd += ['-enable-kvm', '-cpu', 'host']
    open(serial_log, 'wb').close()
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    ok = True
    def wait_for(markers, deadline):
        pending = list(markers)
        while pending and time.time() < deadline and proc.poll() is None:
            log = open(serial_log, 'rb').read().decode(errors='replace')
            pending = [m for m in pending if m not in log]
            if 'KERNEL PANIC' in log or 'CPU EXCEPTION' in log:
                return pending, log, True
            time.sleep(0.2)
        log = open(serial_log, 'rb').read().decode(errors='replace')
        return pending, log, False
    deadline = time.time() + args.timeout
    pending, log, crashed = wait_for(expect, deadline)
    if pending or crashed:
        ok = False
    ser = None
    if ok and (args.keys or args.serial_in):
        if args.serial_in:
            ser = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            ser.connect(serial_sock)
            for ch in args.serial_in.encode():
                ser.sendall(bytes([ch]))
                time.sleep(0.02)
        if args.keys:
            for k in args.keys.split():
                monitor_cmd(mon, f'sendkey {k}')
                time.sleep(0.05)
        if args.after_expect:
            pending, log, crashed = wait_for(args.after_expect, time.time() + args.timeout)
            if pending or crashed:
                ok = False
    if args.screenshot:
        time.sleep(0.5)
        monitor_cmd(mon, 'screendump build/screen.ppm')
        time.sleep(0.5)
        try:
            ppm_to_png('build/screen.ppm', args.screenshot)
            print(f'screenshot: {args.screenshot}')
        except Exception as e:
            print('screenshot failed:', e)
    if ser:
        ser.close()
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
    err = proc.stderr.read().decode(errors='replace')
    log = open(serial_log, 'rb').read().decode(errors='replace')
    print(log[-6000:])
    if err.strip():
        print('qemu stderr:', err[-2000:])
    if ok:
        print('QTEST PASS')
        return 0
    print('QTEST FAIL: missing markers:', pending, 'crashed' if crashed else '')
    return 1

if __name__ == '__main__':
    sys.exit(main())
