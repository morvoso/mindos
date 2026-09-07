# Developing on MindOS

`mindos-dev` (`packages/mindos-dev/`) is the developer stack. It is a meta
package: the tools come from the Arch repositories, and the package adds the
shell prompt, git defaults, kernel limits and a setup command that make them
work together out of the box. Nothing in it runs unless you open a shell.

## What is in it

| Area | Packages |
| --- | --- |
| Compilers and build | base-devel, clang/llvm/lld, cmake, ninja, mold (fast linker), sccache (compile cache), just (task runner) |
| Languages | rustup (Rust), nodejs + npm, python + pip + uv, go |
| Git | git, git-lfs, github-cli, lazygit, git-delta, difftastic |
| Debugging and profiling | gdb, lldb, strace, ltrace, perf, valgrind, hyperfine, tokei |
| Containers | docker + docker-compose, podman, distrobox |
| Editors and terminals | neovim, VS Code (`code`, the Code - OSS build), kitty, tmux |
| Shell | fish, zsh, starship, zoxide, direnv, fzf, shellcheck, man-db + man-pages, tldr |
| Looking at things | ripgrep, fd, bat, eza, jq, sd, dust, duf, btop, bottom, htop, nvtop |

Everything is an Arch package, so `mindos-pkg info NAME` or `pacman -Qi
NAME` says what a tool is and `mindos-pkg install NAME` adds anything not
listed (`docs/PACKAGES.md`).

## First run

![Settings › Developer](img/settings-developer.png)

```
mindos-dev-setup
```

It is idempotent and skips whatever is already done:

* Rust: `rustup default stable` and the `rust-analyzer`, `clippy` and
  `rustfmt` components (rustup keeps toolchains per user, so the package
  cannot do this for you).
* Docker: enables and starts `docker.service` and `docker.socket` and adds
  you to the `docker` group, through `sudo` and saying so first. Log out and
  back in once for the group to apply. Podman is rootless and needs nothing.
  The same two steps have buttons in Settings › Developer, which runs them
  with `pkexec` — the shell's authentication dialog asks for your password,
  no terminal involved.
* VS Code: writes `~/.config/Code - OSS/User/settings.json` if you have none
  (Default Dark Modern theme, JetBrains Mono with ligatures, custom title bar,
  format-on-save off, telemetry off).
* git: asks for `user.name` and `user.email` if they are not set and you are
  at a terminal.

`mindos-dev-setup --status` prints the versions of rustc, cargo, node, npm,
python, go, docker and podman and the Docker state; `--status --json` is what
Settings › Developer reads:

```json
{"tools":[{"name":"rustc","version":"1.98.1"},...,{"name":"go","version":null}],
 "docker":{"active":true,"enabled":true,"member":true}}
```

## The shell

fish is the default shell; `/etc/fish/conf.d/mindos-dev.fish` is sourced by
every interactive fish and sets up:

* **starship** with the MindOS prompt (`/etc/mindos/dev/starship.toml`): a
  line with the directory (cyan), branch (violet), what git wants you to
  notice (amber: `!` modified, `+` staged, `?` untracked, `⇡2` ahead, a
  rebase in progress), the Rust/Node/Python/Go version when you are inside
  such a project, the duration of commands that took 2 s or more, and the
  exit code of a failed command in pink; then a prompt character, green after
  success and pink after a failure. Your own `~/.config/starship.toml` takes
  over if it exists.
* **zoxide**: `z NAME` jumps to a directory you have visited, `zi` picks one
  with fzf.
* **direnv**: a `.envrc` in a project is loaded when you enter it and
  unloaded when you leave (`direnv allow` the first time).
* **fzf**: Ctrl-R searches history, Ctrl-T files, Alt-C directories.
* `ls`, `ll`, `la` are eza with icons and directories first (`ll` adds git
  status); `cat` is bat without a pager; `lg` is lazygit. `command ls` and
  `command cat` run the originals. `EDITOR` is nvim unless you set it.
* `mindos` lists the MindOS commands (the Mind, `mindos-pkg`, `mindos-perf`,
  `mindos-boot`, ...).

zsh gets the same from `/etc/mindos/dev/zshrc`; add to `~/.zshrc`:

```sh
[ -r /etc/mindos/dev/zshrc ] && source /etc/mindos/dev/zshrc
```

bash is untouched. All of these files are in `backup=()`, so edits survive
package upgrades (a changed default arrives as a `.pacnew`).

## git

`/etc/gitconfig` includes `/etc/mindos/dev/gitconfig`; `~/.gitconfig`
overrides both, and `git config --global` never writes to them.

| Setting | Effect |
| --- | --- |
| `init.defaultBranch = main` | new repositories start on `main` |
| `core.pager = delta` | diffs, logs and blame through delta: syntax highlighting (TwoDark), line numbers, `n`/`N` to jump between files, clickable file links; removed lines are tinted pink, added lines green |
| `merge.conflictstyle = zdiff3` | conflicts show the common ancestor too |
| `diff.colorMoved`, `diff.algorithm = histogram` | moved blocks in their own colour, cleaner hunks |
| `pull.rebase`, `rerere.enabled` | pulls rebase instead of merging, and resolved conflicts are remembered |
| `push.autoSetupRemote`, `fetch.prune` | the first push of a branch just works, deleted remote branches disappear |
| `git dft` | a structural diff with difftastic instead of a line diff |

## Containers

* **Docker** for compose stacks and images you ship. `mindos-dev-setup` turns
  the daemon on; before that `docker` fails with "permission denied", which
  is the group membership, not the install.
* **Podman** is installed alongside, rootless, no daemon; `podman` is a
  drop-in for `docker` for most commands.
* **distrobox** gives you another distribution's toolchain in a container
  that shares your home, display and devices, on top of Podman:

  ```
  distrobox create -n ubuntu -i ubuntu:24.04
  distrobox enter ubuntu          # apt install whatever the project wants
  distrobox-export --app code     # optional: put a container app in the menu
  ```

  Inside a box the prompt shows the box name in amber. Use this for anything
  that wants glibc from a specific Ubuntu, a vendor SDK, or a `.deb`; the
  host stays clean.

## Kernel limits

`/usr/lib/sysctl.d/61-mindos-dev.conf` raises the inotify limits
(`max_user_watches` 1048576, `max_user_instances` 1024) so editors, bundlers
and test watchers can watch a large tree, and sets `kernel.perf_event_paranoid
= 1` so `perf record ./my-program` works without root. `kptr_restrict` stays
at the default.

## Ask the Mind

The Mind is a developer tool too. It has the journal, files, package
management and a terminal as tools (`docs/ARCHITECTURE.md`), so from the Mind
bar or a terminal:

```
mind "why does my build fail"            # it re-runs the build, reads the output and the logs, and proposes a fix
mind "install the sdl2 dev package"      # mindos-pkg: repos, then Flathub, then the AUR
mind "what is eating my inotify watches"
mind "show me the last crash of foo.service"
```

Installing packages and changing configuration are *change* actions: the
Mind bar shows the call and you confirm, unless autopilot is on for that
category.

## Where things live

| Path | What |
| --- | --- |
| `/etc/mindos/dev/starship.toml` | the prompt |
| `/etc/fish/conf.d/mindos-dev.fish`, `/etc/mindos/dev/zshrc` | shell integration |
| `/etc/gitconfig` → `/etc/mindos/dev/gitconfig` | git defaults |
| `/usr/lib/sysctl.d/61-mindos-dev.conf` | inotify and perf limits |
| `/usr/bin/mindos-dev-setup` | setup and `--status [--json]` |
| `~/.rustup`, `~/.cargo` | Rust toolchains and binaries (per user) |
| `~/.config/Code - OSS/User/settings.json` | VS Code settings |
| `~/.local/share/containers` | Podman images and distrobox containers |
