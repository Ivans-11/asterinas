#!/usr/bin/env bash
set -euo pipefail

ASTERINAS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_INITRAMFS_DIR="${ASTERINAS_ROOT}/test/initramfs"
TEST_INITRAMFS_BUILD_DIR="${TEST_INITRAMFS_DIR}/build"
TGOSKITS_ROOT="${AXVISOR_TGOSKITS_ROOT:-${ASTERINAS_ROOT}/../tgoskits}"
SETUP_QEMU_SCRIPT="${TGOSKITS_ROOT}/os/axvisor/scripts/setup_qemu.sh"
DEFAULT_VDSO_DIR="${ASTERINAS_ROOT}/../linux_vdso"
LOCAL_OSDK_BIN="${ASTERINAS_ROOT}/osdk/target/debug/cargo-osdk"
LOCAL_OSDK_MANIFEST="${ASTERINAS_ROOT}/osdk/Cargo.toml"

MODE="run"
TARGET_ARCH=""
GUEST=""
DEFAULT_TARGET_ARCH="x86_64"

usage() {
  cat <<'EOF'
Usage:
  tools/axvisor_run.sh [prepare|build|run] [--target-arch ARCH] [--guest GUEST]

Default target architecture: x86_64

If `--guest` is omitted, the script runs only the Asterinas-hosted Axvisor
runtime. If `--guest` is given, it also prepares and boots the selected guest.

Supported target architectures:
  x86_64
  riscv64
  loongarch64

Supported guests:
  nimbos
  nimbos-uefi
  linux-x86_64-uefi

Environment overrides:
  VDSO_LIBRARY_DIR         vDSO artifact directory
  AXVISOR_SCHEME           override the OSDK scheme
  AXVISOR_TGOSKITS_ROOT    sibling tgoskits checkout
  AXVISOR_EXTRA_QEMU_ARGS  extra raw QEMU arguments appended to `cargo osdk run`
  AXVISOR_SLIM_INITRAMFS=0|1
                           override the private initramfs slimming step
  AXVISOR_USE_INSTALLED_OSDK=1
                           force use of installed `cargo osdk` instead of repo-local OSDK
EOF
}

ensure_nix_in_path() {
  if command -v nix-build >/dev/null 2>&1; then
    return
  fi

  local nix_profile_sh="${HOME}/.nix-profile/etc/profile.d/nix.sh"
  if [ -f "${nix_profile_sh}" ]; then
    # shellcheck disable=SC1090
    . "${nix_profile_sh}"
  fi

  if ! command -v nix-build >/dev/null 2>&1; then
    echo "missing nix-build in PATH; source ${nix_profile_sh} or install Nix correctly" >&2
    exit 1
  fi
}

default_target_arch() {
  echo "${DEFAULT_TARGET_ARCH}"
}

scheme_for_arch() {
  case "$1" in
    x86_64)
      echo "axvisor-x86_64"
      ;;
    riscv64)
      echo "axvisor-riscv64"
      ;;
    loongarch64)
      echo "axvisor-loongarch64"
      ;;
    *)
      echo "unsupported target arch: $1" >&2
      exit 1
      ;;
  esac
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

initramfs_out_link() {
  echo "${TEST_INITRAMFS_BUILD_DIR}/${TARGET_ARCH}/initramfs.cpio.gz"
}

should_slim_initramfs() {
  if [ -n "${AXVISOR_SLIM_INITRAMFS:-}" ]; then
    [ "${AXVISOR_SLIM_INITRAMFS}" = "1" ]
    return
  fi

  [ "${TARGET_ARCH}" = "riscv64" ]
}

slim_initramfs_artifact() {
  echo "${TEST_INITRAMFS_BUILD_DIR}/${TARGET_ARCH}/axvisor-initramfs.cpio.gz"
}

build_slim_initramfs() {
  local arch_dir artifact source_root tmp_artifact

  arch_dir="${TEST_INITRAMFS_BUILD_DIR}/${TARGET_ARCH}"
  artifact="$(slim_initramfs_artifact)"
  tmp_artifact="${artifact}.tmp"
  source_root="$(readlink -f "${TEST_INITRAMFS_BUILD_DIR}/initramfs")"

  mkdir -p "${arch_dir}"
  rm -f "${tmp_artifact}"

  (
    cd "${source_root}"
    find . \
      \( \
        -path './nix/store/*/share/i18n' -o -path './nix/store/*/share/i18n/*' -o \
        -path './nix/store/*/share/locale' -o -path './nix/store/*/share/locale/*' -o \
        -path './nix/store/*/lib/gconv' -o -path './nix/store/*/lib/gconv/*' -o \
        -path './nix/store/*/lib/locale' -o -path './nix/store/*/lib/locale/*' \
      \) -prune -o -print0 | \
      LC_ALL=C sort -z | cpio --null -o -H newc --quiet | gzip -n > "${tmp_artifact}"
  )

  mv "${tmp_artifact}" "${artifact}"
}

ensure_initramfs_links() {
  local arch_dir out_link store_path

  arch_dir="${TEST_INITRAMFS_BUILD_DIR}/${TARGET_ARCH}"
  out_link="$(initramfs_out_link)"
  mkdir -p "${arch_dir}"

  if should_slim_initramfs; then
    store_path="$(slim_initramfs_artifact)"
  else
    store_path="$(readlink -f "${TEST_INITRAMFS_BUILD_DIR}/initramfs.cpio.gz")"
  fi
  ln -sfn "${store_path}" "${out_link}"
}

run_osdk() {
  if [ "${AXVISOR_USE_INSTALLED_OSDK:-0}" = "1" ]; then
    cargo osdk "$@"
    return
  fi

  if [ -x "${LOCAL_OSDK_BIN}" ] && ! find "${ASTERINAS_ROOT}/osdk/src" \
    "${ASTERINAS_ROOT}/osdk/Cargo.toml" \
    "${ASTERINAS_ROOT}/osdk/Cargo.lock" \
    -type f -newer "${LOCAL_OSDK_BIN}" | grep -q .; then
    "${LOCAL_OSDK_BIN}" osdk "$@"
    return
  fi

  if [ -f "${LOCAL_OSDK_MANIFEST}" ]; then
    OSDK_LOCAL_DEV=1 cargo run --manifest-path "${LOCAL_OSDK_MANIFEST}" -- osdk "$@"
    return
  fi

  echo "missing usable cargo-osdk; install one or build ${LOCAL_OSDK_BIN}" >&2
  exit 1
}

prepare_initramfs() {
  ensure_nix_in_path
  (cd "${TEST_INITRAMFS_DIR}" && make TARGET_ARCH="${TARGET_ARCH}" BENCHMARK=none build)
  if should_slim_initramfs; then
    build_slim_initramfs
  fi
  ensure_initramfs_links
}

prepare_guest_assets() {
  if [ ! -x "${SETUP_QEMU_SCRIPT}" ]; then
    echo "missing AxVisor guest setup script: ${SETUP_QEMU_SCRIPT}" >&2
    exit 1
  fi

  (cd "${TGOSKITS_ROOT}/os/axvisor" && "${SETUP_QEMU_SCRIPT}" "${GUEST}")
}

guest_vmconfig_path() {
  generated_vmconfig_path "${GUEST}"
}

guest_rootfs_path() {
  echo "${TGOSKITS_ROOT}/os/axvisor/tmp/rootfs.img"
}

guest_qemu_args() {
  local rootfs qemu_args

  rootfs="$(guest_rootfs_path)"
  qemu_args="-smp 2 -device virtio-blk-pci,drive=disk0 -drive id=disk0,if=none,format=raw,file=${rootfs}"

  if [ -n "${AXVISOR_EXTRA_QEMU_ARGS:-}" ]; then
    qemu_args="${qemu_args} ${AXVISOR_EXTRA_QEMU_ARGS}"
  fi

  echo "${qemu_args}"
}

run_axvisor() {
  local scheme initramfs vdso_dir vmconfig qemu_args rootfs

  scheme="${AXVISOR_SCHEME:-$(scheme_for_arch "${TARGET_ARCH}")}"
  initramfs="$(initramfs_out_link)"
  vdso_dir="${VDSO_LIBRARY_DIR:-${DEFAULT_VDSO_DIR}}"
  vmconfig=""
  qemu_args="${AXVISOR_EXTRA_QEMU_ARGS:-}"

  if [ ! -f "${initramfs}" ]; then
    echo "missing prepared initramfs: ${initramfs}" >&2
    echo "run \`tools/axvisor_run.sh prepare${TARGET_ARCH:+ --target-arch ${TARGET_ARCH}}${GUEST:+ --guest ${GUEST}}\` first" >&2
    exit 1
  fi
  if [ ! -d "${vdso_dir}" ]; then
    echo "missing VDSO_LIBRARY_DIR: ${vdso_dir}" >&2
    exit 1
  fi

  if [ -n "${GUEST}" ]; then
    vmconfig="$(guest_vmconfig_path)"
    rootfs="$(guest_rootfs_path)"
    qemu_args="$(guest_qemu_args)"

    if [ ! -f "${vmconfig}" ]; then
      echo "missing generated VM config: ${vmconfig}" >&2
      exit 1
    fi
    if [ ! -f "${rootfs}" ]; then
      echo "missing guest rootfs image: ${rootfs}" >&2
      exit 1
    fi
  fi

  echo "MODE=${MODE}"
  echo "OSDK_TARGET_ARCH=${TARGET_ARCH}"
  echo "AXVISOR_SCHEME=${scheme}"
  echo "VDSO_LIBRARY_DIR=${vdso_dir}"
  echo "INITRAMFS=${initramfs}"
  if [ -n "${vmconfig}" ]; then
    echo "AXVISOR_VM_CONFIGS=${vmconfig}"
  fi
  if [ -n "${qemu_args}" ]; then
    echo "QEMU_ARGS=${qemu_args}"
  fi

  if [ "${MODE}" = "build" ]; then
    if [ -n "${vmconfig}" ]; then
      (cd "${ASTERINAS_ROOT}" && AXVISOR_VM_CONFIGS="${vmconfig}" OSDK_TARGET_ARCH="${TARGET_ARCH}" \
        VDSO_LIBRARY_DIR="${vdso_dir}" run_osdk build --scheme "${scheme}" --features axvisor \
        --initramfs "${initramfs}")
      return
    fi

    (cd "${ASTERINAS_ROOT}" && OSDK_TARGET_ARCH="${TARGET_ARCH}" VDSO_LIBRARY_DIR="${vdso_dir}" \
      run_osdk build --scheme "${scheme}" --features axvisor --initramfs "${initramfs}")
    return
  fi

  if [ -n "${vmconfig}" ]; then
    if [ -n "${qemu_args}" ]; then
      (cd "${ASTERINAS_ROOT}" && AXVISOR_VM_CONFIGS="${vmconfig}" OSDK_TARGET_ARCH="${TARGET_ARCH}" \
        VDSO_LIBRARY_DIR="${vdso_dir}" run_osdk run --scheme "${scheme}" --features axvisor \
        --initramfs "${initramfs}" --qemu-args="${qemu_args}")
      return
    fi

    (cd "${ASTERINAS_ROOT}" && AXVISOR_VM_CONFIGS="${vmconfig}" OSDK_TARGET_ARCH="${TARGET_ARCH}" \
      VDSO_LIBRARY_DIR="${vdso_dir}" run_osdk run --scheme "${scheme}" --features axvisor \
      --initramfs "${initramfs}")
    return
  fi

  if [ -n "${qemu_args}" ]; then
    (cd "${ASTERINAS_ROOT}" && OSDK_TARGET_ARCH="${TARGET_ARCH}" VDSO_LIBRARY_DIR="${vdso_dir}" \
      run_osdk run --scheme "${scheme}" --features axvisor --initramfs "${initramfs}" \
      --qemu-args="${qemu_args}")
    return
  fi

  (cd "${ASTERINAS_ROOT}" && OSDK_TARGET_ARCH="${TARGET_ARCH}" VDSO_LIBRARY_DIR="${vdso_dir}" \
    run_osdk run --scheme "${scheme}" --features axvisor --initramfs "${initramfs}")
}

while [ $# -gt 0 ]; do
  case "$1" in
    prepare|build|run)
      MODE="$1"
      shift
      ;;
    --target-arch)
      shift
      [ $# -gt 0 ] || usage
      TARGET_ARCH="$1"
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

if [ -n "${GUEST}" ] && [ -n "${TARGET_ARCH}" ] && [ "${TARGET_ARCH}" != "x86_64" ]; then
  echo "guest mode currently supports only --target-arch x86_64; got ${TARGET_ARCH}" >&2
  exit 1
fi

if [ -z "${TARGET_ARCH}" ]; then
  TARGET_ARCH="$(default_target_arch)"
fi

if [ -n "${GUEST}" ]; then
  prepare_guest_assets
fi

prepare_initramfs

if [ "${MODE}" != "prepare" ]; then
  run_axvisor
fi
