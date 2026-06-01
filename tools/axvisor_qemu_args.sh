#!/usr/bin/env bash

set -euo pipefail

SCHEME="${1:-}"
MEMORY="${MEM:-8G}"
CPUS="${SMP:-1}"

usage() {
  echo "Usage: tools/axvisor_qemu_args.sh axvisor-{x86_64|riscv64|loongarch64}" >&2
  exit 1
}

[ -n "${SCHEME}" ] || usage

case "${SCHEME}" in
  axvisor-x86_64)
    cat <<EOF
-machine q35,kernel-irqchip=split \
-enable-kvm \
-cpu host \
-smp ${CPUS} \
-m ${MEMORY} \
--no-reboot \
-nographic \
-display none \
-serial chardev:mux \
-monitor chardev:mux \
-chardev stdio,id=mux,mux=on,signal=off,logfile=qemu.log \
-device isa-debug-exit,iobase=0xf4,iosize=0x04
EOF
    ;;
  axvisor-riscv64)
    cat <<EOF
-cpu rv64,h=true,svpbmt=true,zkr=true \
-machine virt \
-m ${MEMORY} \
-smp ${CPUS} \
--no-reboot \
-nographic \
-display none \
-monitor chardev:mux \
-chardev stdio,id=mux,mux=on,signal=off,logfile=qemu.log \
-drive if=none,format=raw,id=x0,file=./test/initramfs/build/ext2.img \
-drive if=none,format=raw,id=x1,file=./test/initramfs/build/exfat.img \
-device virtio-blk-device,drive=x1 \
-device virtio-blk-device,drive=x0 \
-device virtio-keyboard-device \
-device virtio-serial-device \
-device virtconsole,chardev=mux \
-serial file:qemu-serial.log
EOF
    ;;
  axvisor-loongarch64)
    cat <<EOF
-machine virt \
-m ${MEMORY} \
-smp ${CPUS} \
--no-reboot \
-nographic \
-display none \
-serial chardev:mux \
-monitor chardev:mux \
-chardev stdio,id=mux,mux=on,signal=off,logfile=qemu.log \
-device virtio-keyboard-pci \
-device virtio-serial \
-device virtconsole,chardev=mux \
-rtc base=utc
EOF
    ;;
  *)
    echo "unsupported Axvisor scheme: ${SCHEME}" >&2
    exit 1
    ;;
esac
