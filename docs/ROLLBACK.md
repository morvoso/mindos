# Updates and the way back

MindOS is a rolling Arch system, so an update can break something. The
answer is that every pacman run is bracketed by snapshots of the system, every
snapshot is in the boot menu, and one command makes a snapshot the system
again. Nothing needs to be installed or set up; the installer does it all.

## The pieces

| Piece | Package | What it does |
| --- | --- | --- |
| Limine | `limine` (Arch) | The boot menu, on the EFI system partition (`/boot`) and in the BIOS boot partition. White on MindOS red, then the kernel console continues in the same colours. |
| snapper | `snapper` (Arch) | Read-only btrfs snapshots of the root subvolume `@`, stored in the `@snapshots` subvolume at `/.snapshots`. The `root` configuration ships in `mindos-base` (`/etc/snapper/configs/root`): no timeline snapshots, keep the newest 10 and the 5 newest "important" ones, wheel members can list them. `snapper-cleanup.timer` prunes. |
| snap-pac | `snap-pac` (Arch) | pacman hooks that take a `pre` snapshot before and a `post` snapshot after every transaction. Kernel, graphics stack, desktop and `pacman -Syu` count as important. |
| `mindos-boot` | `mindos-base` | Writes `/boot/limine.conf`, keeps a copy of the kernel and initramfs that belong to each snapshot on `/boot`, lists the snapshots in the menu, restores one. Runs from a snapper plugin (`/usr/lib/snapper/plugins/10-mindos-boot`) whenever a snapshot is created or deleted, and from a pacman hook (`95-mindos-boot`) when a kernel, the microcode, Limine itself, the command line or the theme changes. |

`/home`, `/var/log` and the pacman package cache are separate subvolumes
(`@home`, `@log`, `@pkg`), so a snapshot is the system alone: restoring one
never touches your files, and the logs of the boot that went wrong survive.

## What happens on an update

```
sudo pacman -Syu
  05-snap-pac-pre     snapper create (pre)   → plugin: mindos-boot sync   (kernel copy for #N)
  … packages …
  90-mkinitcpio       new initramfs on /boot
  95-mindos-boot      /boot/limine.conf rewritten (only when a kernel or Limine changed)
  zz-snap-pac-post    snapper create (post)  → plugin: mindos-boot sync   (kernel copy for #N+1)
```

`/boot` is a FAT partition outside the snapshot, so `mindos-boot sync` copies
the kernel and initramfs that were current when the snapshot was taken into
`/boot/mindos/k/<id>/`, one copy per distinct kernel+initramfs (most snapshots
share one), and notes the id in `/boot/mindos/snapshots/<N>`. Copies whose
snapshot snapper has cleaned up are removed on the next sync. About 35 MB per
distinct kernel on a 1 GiB partition.

## The boot menu

```
    MindOS                      linux-mindos, / on @
    MindOS (linux)              any other kernel found on /boot
[+] Snapshots
    ├─ #12  2026-09-06 22:48  after: pacman -Syu
    ├─ #11  2026-09-06 22:47  before: pacman -Syu
    └─ #1   2026-09-06 20:10  MindOS installed
```

A snapshot entry boots the snapshot's own kernel with
`rootflags=subvol=@snapshots/N/snapshot systemd.volatile=overlay`: the
read-only snapshot with a RAM overlay on top (`sd-volatile` in the initramfs),
so the desktop comes up writable and everything works, but nothing done there
outlasts the reboot. `mindos-boot status` shows "snapshot #N, changes in RAM"
while booted that way. The menu shows the newest 12 (`SNAPSHOT_ENTRIES` in
`/etc/mindos/boot.conf`).

## Rolling back

When restoring from the running system, the old root stays writable until
reboot so applications can finish normally. On the next boot,
`mindos-snapshot-seal.service` makes retained MindOS roots read-only before
scheduled cleanup. It defers any root that is mounted (including a snapshot
boot's lower filesystem or a bind-mounted child), received, or the filesystem's
default subvolume. It changes no child flags or contents. The service is included
in `mindos-base` 0.1.0-14 and later; a restored system needs that version for
automatic sealing. Its status and journal show any failure.

Roots retained by `mindos-boot restore` can contain the empty Btrfs
subvolumes systemd creates at `var/lib/machines` and `var/lib/portables`.
Before Snapper deletes such a root, `snapshot-prune` removes those empty
children. It only handles MindOS restore snapshots, freezes children before
checking their contents, and preserves read-only flags on failure. Mounted,
writable, received or default roots, unfamiliar child subvolumes and children
containing data are left intact. This prevents ordinary cleanup from deleting
container or portable-image data that was never included in a recursive backup.

`scripts/tests/test_snapshot_prune.py` verifies this against a temporary
loop-mounted Btrfs image; run it as root in the development VM. It creates
its own filesystem and never uses the installed root as a test fixture.
See the upstream [Btrfs subvolume documentation](https://btrfs.readthedocs.io/en/latest/btrfs-subvolume.html)
for the distinction between a moved root and a non-recursive snapshot.

1. Reboot, open **Snapshots** in the menu, boot the "before" snapshot of the
   update that broke things. Check that it is the state you want.
2. `sudo mindos-boot restore` (the booted snapshot; or `restore N` from any
   boot). The current `@` becomes a new snapper snapshot ("the system before
   restoring #N", so the restore itself can be undone), the chosen snapshot is
  copied to a fresh writable `@`, the matching kernel and initramfs go back to
   the main entry on `/boot`, and the menu is rewritten.
3. Reboot into **MindOS**.

Package snapshots include pacman's lock file because the transaction is still
running when snap-pac takes them. Restore removes that copied lock from the
new, unused root so package installation works after reboot. The running root's
lock is preserved. A package database relocated through a symlink is left
alone, since it may refer to a database still in use outside the restored root.

The packages that were rolled back are still in the pacman cache and the
database is the old one, so `pacman -Syu` simply offers the update again;
`pacman -Syu --ignore <pkg>` holds the culprit.

## Commands

```
mindos-boot status          firmware, Limine, kernel, command line, what is booted
mindos-boot list            snapshots, their kernel copy, the booted one marked *
mindos-boot restore [N]     make snapshot N the system again (default: the booted one)
mindos-boot config          rewrite /boot/limine.conf (after editing /etc/mindos/boot.conf)
mindos-boot sync            kernel copies for every snapshot, then config
mindos-boot install         Limine binaries onto /boot and the BIOS boot partition, UEFI entry
snapper -c root list        the snapshots (wheel members need no sudo)
snapper -c root create -d "before I try X"      a manual snapshot (lands in the menu at once)
snapper -c root delete N    remove one (its kernel copy goes with it)
```

`/etc/mindos/boot.conf`: `TIMEOUT` (1 s), `CMDLINE` (replaces the packaged
kernel command line in `/usr/share/mindos/kernel-cmdline`), `CMDLINE_EXTRA`
(appended), `SNAPSHOT_ENTRIES`. The `nvidia_drm.*` switches are dropped by
themselves on machines without an NVIDIA card. The colours come from
`/usr/share/mindos/limine-theme.conf` (`mindos-theme`).

## Verified

Dev VM, 2026-09-06: the Limine menu (`docs/img/limine-red.png`,
`docs/img/limine-snapshots.png`); snapshot #3 booted with the overlay
(`findmnt /` shows `overlay`, `/home` and `/.snapshots` the real
subvolumes); `mindos-boot restore 1` from inside that boot brought GRUB back
(snapshot #1 predates its removal), `restore 4` removed it again; deleting a
snapshot with snapper pruned its menu entry and kernel copy; reinstalling
`limine` re-ran the install through the pacman hook.

The installer and restore command remove numeric `subvolid` options when a
Btrfs mount already names its subvolume with `subvol=`. Restoring creates a
new numeric ID; the named path remains stable. Custom mounts that specify
only an ID are preserved. This prevents generated `fstab` entries from
pinning a restored root to its old subvolume.
