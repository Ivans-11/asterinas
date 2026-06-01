#!/usr/bin/env bash
set -euo pipefail

ASTERINAS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TGOSKITS_ROOT="${AXVISOR_TGOSKITS_ROOT:-${ASTERINAS_ROOT}/../tgoskits}"
SETUP_QEMU_SCRIPT="${TGOSKITS_ROOT}/os/axvisor/scripts/setup_qemu.sh"
DEFAULT_VDSO_DIR="${ASTERINAS_ROOT}/../linux_vdso"
LOCAL_OSDK_BIN="${ASTERINAS_ROOT}/osdk/target/debug/cargo-osdk"

MODE="run"
GUEST="nimbos"

usage() {
  cat <<'EOF'
Usage: tools/axvisor_guest_probe.sh [prepare|build|run] [--guest GUEST]

Supported guests:
  nimbos
  nimbos-uefi
  linux-x86_64-uefi

Environment overrides:
  AXVISOR_TGOSKITS_ROOT   sibling tgoskits checkout
  VDSO_LIBRARY_DIR        vDSO artifact directory
  AXVISOR_EXTRA_QEMU_ARGS extra raw QEMU arguments appended to the probe run
EOF
}

generated_vmconfig_path() {
  case "$1" in
    nimbos)
      echo "${TGOSKITS_ROOT}/os/axvisor/tmp/vmconfigs/nimbos-x86_64-qemu-smp1.generated.toml"
      ;;
    nimbos-uefi)
      echo "${TGOSKITS_ROOT}/os/axvisor/tmp/vmconfigs/nimbos-x86_64-qemu-uefi-smp1.generated.toml"
      ;;
    linux-x86_64-uefi)
      echo "${TGOSKITS_ROOT}/os/axvisor/tmp/vmconfigs/linux-x86_64-qemu-uefi-smp1.generated.toml"
      ;;
    *)
      echo "unsupported guest: $1" >&2
      exit 1
      ;;
  esac
}

run_osdk() {
  if [ -x "${LOCAL_OSDK_BIN}" ]; then
    "${LOCAL_OSDK_BIN}" osdk "$@"
  else
    cargo osdk "$@"
  fi
}

prepare_guest_assets() {
  if [ ! -x "${SETUP_QEMU_SCRIPT}" ]; then
    echo "missing AxVisor guest setup script: ${SETUP_QEMU_SCRIPT}" >&2
    exit 1
  fi

  (cd "${TGOSKITS_ROOT}/os/axvisor" && "${SETUP_QEMU_SCRIPT}" "${GUEST}")
}

run_probe() {
  local vmconfig rootfs vdso_dir qemu_args

  vmconfig="$(generated_vmconfig_path "${GUEST}")"
  rootfs="${TGOSKITS_ROOT}/os/axvisor/tmp/rootfs.img"
  vdso_dir="${VDSO_LIBRARY_DIR:-${DEFAULT_VDSO_DIR}}"
  qemu_args="-smp 2 -device virtio-blk-pci,drive=disk0 -drive id=disk0,if=none,format=raw,file=${rootfs}"

  if [ ! -f "${vmconfig}" ]; then
    echo "missing generated VM config: ${vmconfig}" >&2
    exit 1
  fi
  if [ ! -f "${rootfs}" ]; then
    echo "missing guest rootfs image: ${rootfs}" >&2
    exit 1
  fi
  if [ ! -d "${vdso_dir}" ]; then
    echo "missing VDSO_LIBRARY_DIR: ${vdso_dir}" >&2
    exit 1
  fi

  if [ -n "${AXVISOR_EXTRA_QEMU_ARGS:-}" ]; then
    qemu_args="${qemu_args} ${AXVISOR_EXTRA_QEMU_ARGS}"
  fi

  echo "AXVISOR_VM_CONFIGS=${vmconfig}"
  echo "VDSO_LIBRARY_DIR=${vdso_dir}"
  echo "QEMU_ARGS=${qemu_args}"

  if [ "${MODE}" = "build" ]; then
    (cd "${ASTERINAS_ROOT}" && AXVISOR_VM_CONFIGS="${vmconfig}" VDSO_LIBRARY_DIR="${vdso_dir}" \
      run_osdk build --scheme axvisor-probe --features axvisor)
    return
  fi

  (cd "${ASTERINAS_ROOT}" && AXVISOR_VM_CONFIGS="${vmconfig}" VDSO_LIBRARY_DIR="${vdso_dir}" \
    run_osdk run --scheme axvisor-probe --features axvisor --qemu-args="${qemu_args}")
}

while [ $# -gt 0 ]; do
  case "$1" in
    prepare|build|run)
      MODE="$1"
      shift
      ;;
    --guest)
      shift
      [ $# -gt 0 ] || usage
      GUEST="$1"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

prepare_guest_assets

if [ "${MODE}" != "prepare" ]; then
  run_probe
fi
