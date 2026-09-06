#!/usr/bin/env python3
"""Build a MindOS initrd image.

Format (little endian):
  header (32 bytes): magic "MINDINIT", version u32, count u32, strtab_off u32,
                     strtab_size u32, entries_off u32, reserved u32
  entries (32 bytes each): path_off u32, path_len u32, data_off u64, size u64,
                     mode u32, kind u32   (kind: 0 file, 1 dir, 2 symlink)
  string table, then file data (each file 4096-byte aligned so it can be
  mapped directly into processes without copying).
"""
import os, struct, sys

MAGIC = b"MINDINIT"

def build(root_dirs, extra_files, out_path):
    entries = []  # (path, kind, mode, data)
    seen = set()
    def add_dir(path):
        parts = path.strip('/').split('/')
        for i in range(1, len(parts) + 1):
            p = '/'.join(parts[:i])
            if p and p not in seen:
                seen.add(p)
                entries.append((p, 1, 0o755, b""))
    for root in root_dirs:
        if not os.path.isdir(root):
            continue
        for dirpath, dirnames, filenames in os.walk(root):
            rel = os.path.relpath(dirpath, root)
            rel = '' if rel == '.' else rel
            if rel:
                add_dir(rel)
            dirnames.sort()
            for f in sorted(filenames):
                full = os.path.join(dirpath, f)
                p = os.path.join(rel, f) if rel else f
                if p in seen:
                    continue
                seen.add(p)
                if os.path.islink(full):
                    entries.append((p, 2, 0o777, os.readlink(full).encode()))
                else:
                    st = os.stat(full)
                    entries.append((p, 0, st.st_mode & 0o7777, open(full, 'rb').read()))
    for dest, src in extra_files:
        d = os.path.dirname(dest)
        if d:
            add_dir(d)
        if dest in seen:
            continue
        seen.add(dest)
        st = os.stat(src)
        entries.append((dest, 0, st.st_mode & 0o7777, open(src, 'rb').read()))
    entries.sort(key=lambda e: e[0])

    strtab = bytearray()
    ent_bin = bytearray()
    header_size = 32
    entries_off = header_size
    entries_size = 32 * len(entries)
    strtab_off = entries_off + entries_size
    # compute string table first
    path_offs = []
    for p, kind, mode, data in entries:
        path_offs.append(len(strtab))
        strtab += p.encode() + b"\0"
    data_start = (strtab_off + len(strtab) + 4095) & ~4095
    blob = bytearray()
    for i, (p, kind, mode, data) in enumerate(entries):
        off = data_start + len(blob)
        ent_bin += struct.pack('<IIQQII', path_offs[i], len(p.encode()), off, len(data), mode, kind)
        blob += data
        pad = (-len(blob)) % 4096
        blob += b"\0" * pad
    header = MAGIC + struct.pack('<IIIIII', 1, len(entries), strtab_off, len(strtab), entries_off, 0)
    out = bytearray(header) + ent_bin + strtab
    out += b"\0" * (data_start - len(out))
    out += blob
    with open(out_path, 'wb') as f:
        f.write(out)
    total = sum(len(e[3]) for e in entries)
    print(f"initrd: {len(entries)} entries, {total} bytes payload -> {out_path} ({len(out)} bytes)")

if __name__ == '__main__':
    out = sys.argv[1] if len(sys.argv) > 1 else 'build/initrd.img'
    os.makedirs(os.path.dirname(out) or '.', exist_ok=True)
    roots = ['build/initrd-root']
    extra = []
    build(roots, extra, out)
