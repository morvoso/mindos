# MindOS top-level build. Everything runs inside the Docker build box
# (scripts/buildbox.sh); the host only needs Docker, QEMU and Python (+PIL).

SHELL       := /bin/bash
.SHELLFLAGS := -o pipefail -c
BUILDBOX    := scripts/buildbox.sh
# Build order matters only for dependencies at install time, not for makepkg.
PKGS        := mindos-mind mindwm mindos-theme mindos-base mindos-session mindos-gaming mindos-dev mindos-install
REPO        := build/repo
ISO_PROFILE := build/iso-profile
ISO_OUT     := build/out
# Every ISO build gets a fresh file name (mindos-<date>-r<N>-x86_64.iso) so a build never
# replaces an image that is currently booted in QEMU.  Override with `make iso ISO_REV=7`.
ISO_REV     ?= $(shell n=$$(ls $(ISO_OUT)/*.iso 2>/dev/null | wc -l); echo $$((n + 1)))

.PHONY: buildbox kernel packages repo iso-stage iso qemu qemu-bios screenshot qemu-stop vm vm-install vm-snapshot vm-console clean help

help:
	@echo "targets: buildbox kernel packages repo iso qemu qemu-bios screenshot qemu-stop clean"
	@echo "dev VM:  vm (create on libvirt from the newest ISO) vm-install vm-snapshot NAME=... vm-console"

buildbox:
	$(BUILDBOX) --build

# The kernel takes ~20 minutes on 8 cores; it is separate from `packages`.
kernel:
	mkdir -p build/logs
	$(BUILDBOX) bash -c 'cd packages/linux-mindos && makepkg -sf --noconfirm --skippgpcheck'

# Rust packages resolve their deps with -s; the arch=any packages are only files (-d).
packages:
	mkdir -p build/logs build/cargo-home
	for p in $(PKGS); do \
	  case $$p in \
	    mindos-mind|mindwm) opts="-sf" ;; \
	    *) opts="-fd" ;; \
	  esac; \
	  $(BUILDBOX) bash -c "cd packages/$$p && makepkg $$opts --noconfirm --skippgpcheck" || exit 1; \
	done

repo:
	rm -rf $(REPO)
	mkdir -p $(REPO)
	cp build/packages/*.pkg.tar.zst $(REPO)/
	$(BUILDBOX) bash -c 'cd $(REPO) && repo-add -q mindos.db.tar.zst *.pkg.tar.zst'

# Stage the profile with the local repo and the bundled model inside the airootfs.
iso-stage:
	mkdir -p $(ISO_PROFILE) $(ISO_OUT) build/logs
	rsync -a --delete iso/ $(ISO_PROFILE)/
	mkdir -p $(ISO_PROFILE)/airootfs/var/lib/mindos/repo $(ISO_PROFILE)/airootfs/var/lib/mindos/models
	rsync -a --delete $(REPO)/ $(ISO_PROFILE)/airootfs/var/lib/mindos/repo/
	cp models/*.gguf $(ISO_PROFILE)/airootfs/var/lib/mindos/models/
	sed -i 's|^iso_version=.*|iso_version="$(shell date +%Y.%m.%d)-r$(ISO_REV)"|' $(ISO_PROFILE)/profiledef.sh
	@grep '^iso_version=' $(ISO_PROFILE)/profiledef.sh

# mkarchiso needs root and a work dir with xattr support: keep it inside the container.
iso: iso-stage
	$(BUILDBOX) --root bash -c 'rm -rf /var/tmp/mindos-iso-work && mkarchiso -v -w /var/tmp/mindos-iso-work -o /work/$(ISO_OUT) /work/$(ISO_PROFILE)' 2>&1 | tee build/logs/iso.log
	@echo "ISO written: $(ISO_OUT)/mindos-$(shell date +%Y.%m.%d)-r$(ISO_REV)-x86_64.iso"; ls -la $(ISO_OUT)/*.iso

ISO := $(shell ls -t $(ISO_OUT)/*.iso 2>/dev/null | head -1)

qemu:
	scripts/qemu-iso.sh $(ISO)

qemu-bios:
	scripts/qemu-iso.sh $(ISO) --bios

screenshot:
	scripts/qemu-screenshot.sh build/qemu/screen.png

qemu-stop:
	scripts/qemu-iso.sh --stop

# Persistent development VM on libvirt/virt-manager (docs/DEV-VM.md)
vm:
	scripts/vm/mindos-vm.sh create $(ISO)

vm-install:
	scripts/vm/mindos-vm.sh install

vm-snapshot:
	scripts/vm/mindos-vm.sh snapshot $(NAME) $(DESC)

vm-console:
	scripts/vm/mindos-vm.sh console

clean:
	rm -rf build/makepkg $(ISO_PROFILE)
