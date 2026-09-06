# MindOS top-level build
#   make            build everything and the disk image
#   make run        boot in QEMU (serial on stdio, no graphics window)
#   make run-vnc    boot in QEMU with a VNC display on :0 (connect with a VNC viewer)
#   make test       boot headless and check boot markers
#   make clean

BUILD     := build
KERNEL    := kernel/target/x86_64-unknown-none/release/mindos-kernel
INITRD    := $(BUILD)/initrd.img
DISK      := $(BUILD)/mindos.img
QEMU      ?= qemu-system-x86_64
MEM       ?= 4G
CPUS      ?= 4
QEMU_ARGS ?= -machine q35 -m $(MEM) -smp $(CPUS) -cpu host -enable-kvm \
             -drive file=$(DISK),format=raw,if=none,id=hd0 \
             -device virtio-blk-pci,drive=hd0 \
             -drive file=$(BUILD)/data.img,format=raw,if=none,id=hd1 \
             -device virtio-blk-pci,drive=hd1 \
             -no-reboot -no-shutdown -serial mon:stdio \
             -debugcon file:$(BUILD)/debugcon.log -global isa-debugcon.iobase=0xe9

.PHONY: all kernel user initrd disk run run-vnc test clean

all: disk

kernel:
	cd kernel && cargo build --release

user:
	@if [ -f user/Cargo.toml ]; then cd user && cargo build --release; fi

initrd: user
	python3 tools/mkinitrd.py $(INITRD)

disk: kernel initrd
	scripts/mkdisk.sh $(DISK) $(KERNEL) $(INITRD)

run: disk
	$(QEMU) $(QEMU_ARGS) -display none

run-vnc: disk
	$(QEMU) $(QEMU_ARGS) -display vnc=:0

test: disk
	python3 tools/qtest.py

clean:
	rm -rf $(BUILD) kernel/target user/target host/target
