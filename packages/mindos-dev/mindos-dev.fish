# /etc/fish/conf.d/mindos-dev.fish — the MindOS developer shell (mindos-dev).
# Prompt, directory jumping, per-project environments, fzf keys and a few
# modern replacements for ls/cat. Everything is skipped for non-interactive
# shells, and `command ls` / `command cat` always run the originals.

if status is-interactive
    # Prompt: the MindOS starship theme unless the user has their own.
    if not set -q STARSHIP_CONFIG
        set -l cfg ~/.config
        set -q XDG_CONFIG_HOME; and set cfg $XDG_CONFIG_HOME
        test -e $cfg/starship.toml; or set -gx STARSHIP_CONFIG /etc/mindos/dev/starship.toml
    end
    command -q starship; and starship init fish | source

    command -q zoxide; and zoxide init fish | source          # z DIR, zi
    command -q direnv; and direnv hook fish | source           # .envrc per project
    command -q fzf; and fzf --fish 2>/dev/null | source        # Ctrl-R history, Ctrl-T files, Alt-C dirs

    if command -q eza
        function ls --wraps eza --description 'eza (command ls runs the original)'
            eza --icons=auto --group-directories-first $argv
        end
        function ll --wraps eza --description 'long listing with git status'
            eza --icons=auto --group-directories-first -l --git $argv
        end
        function la --wraps eza --description 'long listing including dotfiles'
            eza --icons=auto --group-directories-first -la --git $argv
        end
    else
        abbr -a ll 'ls -l'
        abbr -a la 'ls -la'
    end
    if command -q bat
        function cat --wraps bat --description 'bat without a pager (command cat runs the original)'
            bat --paging=never $argv
        end
    end
    command -q lazygit; and abbr -a lg lazygit

    if not set -q EDITOR; and command -q nvim
        set -gx EDITOR nvim
    end

    function mindos --description 'list the MindOS developer commands'
        set -l rows \
            'mind "..."         ask the Mind: it can read logs, install packages, fix configs' \
            'mindos-pkg         install/remove/search: MindOS + Arch repos, Flathub, AUR' \
            'mindos-perf        performance modes: status | set balanced|performance|quiet' \
            'mindos-dlss        DLSS presets and overrides for games' \
            'mindos-boot        boot menu and snapshots: list | restore | config' \
            'mindos-dev-setup   Rust toolchain, Docker, VS Code defaults, git identity (--status)'
        for row in $rows
            set -l cmd (string split -f1 ' ' $row)
            if command -q $cmd
                echo $row
            else
                echo (set_color brblack)$row'  (not installed)'(set_color normal)
            end
        end
    end
end
