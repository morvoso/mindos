# MindOS build box: an Arch Linux container with everything needed to build the
# MindOS packages (including the kernel), the package repository and the ISO.
# Root inside the container is what lets makepkg -s and mkarchiso work without
# touching the host.
FROM archlinux:base-devel

# Enable [multilib] (steam and the lib32-* gaming stack live there). The
# container image's pacman.conf has no multilib section at all, so append one.
RUN grep -q '^\[multilib\]' /etc/pacman.conf \
 || printf '\n[multilib]\nInclude = /etc/pacman.d/mirrorlist\n' >> /etc/pacman.conf

RUN pacman -Syu --noconfirm --needed \
      archiso arch-install-scripts \
      bc cpio pahole libelf perl python tar xz zstd xxhash openssl \
      git wget curl rsync namcap pkgconf \
      clang llvm lld rust rust-bindgen rust-src \
      squashfs-tools libisoburn dosfstools mtools e2fsprogs erofs-utils grub syslinux edk2-shell \
      wayland wayland-protocols libinput libxkbcommon mesa libdrm libdisplay-info seatd \
      systemd-libs pixman libglvnd \
      plymouth \
      gtk4 webkitgtk-6.0 gtk4-layer-shell esbuild typescript python-pillow python-fonttools \
 && pacman -Scc --noconfirm

RUN useradd -m -u 1000 -s /bin/bash builder \
 && echo 'builder ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/builder \
 && chmod 440 /etc/sudoers.d/builder

COPY makepkg-mindos.conf /etc/makepkg.conf.d/mindos.conf
COPY entry.sh /usr/local/bin/entry.sh
RUN chmod +x /usr/local/bin/entry.sh

WORKDIR /work
ENTRYPOINT ["/usr/local/bin/entry.sh"]
