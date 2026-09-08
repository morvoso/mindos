# Software on MindOS

Octopi is preinstalled in the live image and installed desktop. Open **Software**
from Mind, the taskbar, or Settings. Search for an app, select Install or Remove,
and Apply to review the transaction and authenticate. Octopi also exposes package
details, files, dependencies, upgrades, the cache cleaner and repository editor.
Its separate update notifier is not autostarted; MindOS keeps one update indicator.

The gaming image ships Steam, Lutris, Wine and the game tuning tools. Discord
and OBS are optional through Software. Developer setup, language toolchains,
containers, shell customizations and CUDA are not part of the default install.
The retained `mindos-dev` source recipe is outside the default build and image.
Community builds are optional (`paru` and `base-devel`); repository packages
and Flatpak work without installing development tools.


MindOS installs software from three sources, always in this order, through a
single command: `mindos-pkg`. The Mind uses it for every install, remove,
search and info request, and it works the same way from a terminal
(`sudo mindos-pkg ...`).

| Source | What it is | Tool underneath | Example |
| --- | --- | --- | --- |
| **MindOS + Arch repositories** | Binary packages: core, extra, multilib and the `[mindos]` repo (kernel, compositor, mind, AUR tools built into the repo such as `paru`) | `pacman` | `discord`, `steam`, `mangohud` |
| **Flathub** | Sandboxed desktop apps, updated independently of the system | `flatpak` (the remote is added on first use) | `spotify`, `obs-studio` (as `com.obsproject.Studio`) |
| **AUR** (disabled by default) | User-submitted packages built locally from source | `paru`, running as the unprivileged `mindos-build` user | `octopi`, `protonup-qt` |

The order reflects trust. The repositories contain signed binary packages
maintained by Arch and MindOS, and Flathub provides sandboxed builds from the
upstream projects. Both are built and signed before they are distributed.

The AUR is different. Its packages are submitted by users and are not
reviewed, and a `PKGBUILD` runs its own shell script on the local system as
part of the installation. For this reason **the AUR is disabled unless the
administrator enables it**:

```
aur = no      # /etc/mindos/pkg.conf, the default
```

* `mindos-pkg install --aur NAME` permits the AUR for that command only.
* `aur = yes` in `/etc/mindos/pkg.conf` permits it permanently.
* `mindos-pkg search` always lists AUR results and notes that the AUR is
  disabled.

Nothing enables the AUR automatically. The Mind never passes `--aur`, and
Settings › Developer installs with `--repo-only`, which stops at the
repositories; the flag is fixed in the shell's command allow-list and cannot
be changed by the page. When a name exists only in the AUR, the installation
stops and reports how to permit it.

## mindos-pkg

```
mindos-pkg install NAME...   repositories → Flathub; reports which source was used
mindos-pkg install --aur     also the AUR, for this command only
mindos-pkg install --repo-only  repositories only; Flathub and the AUR are not used
mindos-pkg remove NAME...    pacman or flatpak, whichever holds the package
mindos-pkg search QUERY      all three sources, short list
mindos-pkg info NAME         source, installation state and details
mindos-pkg where NAME        installed | flatpak-installed | repo | flatpak | aur | none
```

Behaviour:

* Names are plain package names (`discord`, not a description). Flathub
  matches on the app name or the last component of the app id, so `spotify`
  finds `com.spotify.Client`.
* Repository installs refresh package metadata and apply pending system updates
  together with `pacman -Syu --needed`. This avoids stale download URLs and
  partial upgrades. `--repo-only` installs multiple requested packages in one
  transaction. If every requested package is already installed, it reports that
  and makes no changes; use the Updates page to update the system.
* AUR builds run only when the AUR is permitted (above). They run
  `paru -S --needed --noconfirm --skipreview` as `mindos-build`
  (home `/var/lib/mindos/build`, created by systemd-sysusers/tmpfiles). That
  user may run `pacman` through sudo without a password
  (`/etc/sudoers.d/20-mindos-build`) and nothing else. Builds can take minutes;
  the Mind's tool timeout is 30 minutes.
* Output is plain sentences ("octopi: installed from the AUR (0.16.0-1)",
  "foo: not found in the MindOS or Arch repositories, on Flathub, or in the
  AUR", "octopi: available only from the AUR, which is disabled"). The exit
  status is non-zero if any requested package failed, including a package
  refused because it is available only from the AUR.

## How the Mind uses it

`mindd`'s `install_packages`, `remove_packages`, `search_packages` and
`package_info` tools call `mindos-pkg`, and the system prompt explains the
three sources and that a not-found answer means the name exists nowhere (so
the model suggests a search instead of "install it first"). Installing and
removing are *change* actions under the policy layer: the Mind bar shows the
tool call and the user confirms unless autopilot is on for packages.

Ask the Mind "install octopi" and it answers with the source it used; ask
"what is octopi" and it tells you where the package lives before touching
anything.

![The Mind bar installing octopi from the AUR in the dev VM](img/mind-bar-install-octopi.png)

The screenshot is from the dev VM: the earlier attempts above the last one
are from before `mindos-pkg` existed (pacman alone answered "target not
found" and the model misread that as "not installed"); the last one builds
octopi from the AUR in about 95 seconds on 8 vCPUs and reports it.

## Packaging

* `packages/mindos-mind` ships `mindos-pkg`, the `mindos-build` user, its
  home directory and the sudoers rule, and depends on `sudo` and `flatpak`; `paru` and `base-devel` are optional.
* `packages/paru` is the AUR recipe for paru, vendored so the `[mindos]`
  repo can ship it prebuilt against the current pacman (no AUR helper is
  needed to get the AUR helper).
* `packages/mindos-base` depends on `flatpak` so a fresh install can reach
  Flathub without any setup.
