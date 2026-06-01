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
  arceos-riscv64
  linux-riscv64
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
    arceos-riscv64)
      echo "${TGOSKITS_ROOT}/os/axvisor/tmp/vmconfigs/arceos-riscv64-qemu-smp1.generated.toml"
      ;;
    linux-riscv64)
      echo "${TGOSKITS_ROOT}/os/axvisor/tmp/vmconfigs/linux-riscv64-qemu-smp1.generated.toml"
      ;;
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

guest_target_arch() {
  case "$1" in
    arceos-riscv64|linux-riscv64)
      echo "riscv64"
      ;;
    nimbos|nimbos-uefi|linux-x86_64-uefi)
      echo "x86_64"
      ;;
    *)
      echo "unsupported guest: $1" >&2
      exit 1
      ;;
  esac
}

guest_requires_host_rootfs_injection() {
  case "$1" in
    arceos-riscv64)
      return 1
      ;;
    linux-riscv64|nimbos|nimbos-uefi|linux-x86_64-uefi)
      return 0
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

rewrite_vmconfig_array_block() {
  local vmconfig_path="$1"
  local block_name="$2"
  local replacement="$3"
  local tmp_path="${vmconfig_path}.tmp"

  awk -v block_name="${block_name}" -v replacement="${replacement}" '
    BEGIN {
      in_block = 0;
      replaced = 0;
    }
    {
      if (!in_block && $0 ~ ("^" block_name " *= *\\[$")) {
        print replacement;
        in_block = 1;
        replaced = 1;
        next;
      }

      if (in_block) {
        if ($0 ~ /^]$/) {
          print "]";
          in_block = 0;
        }
        next;
      }

      print $0;
    }
    END {
      if (!replaced) {
        exit 1;
      }
      if (in_block) {
        exit 1;
      }
    }
  ' "${vmconfig_path}" > "${tmp_path}" || {
    rm -f "${tmp_path}"
    echo "failed to rewrite ${block_name} in ${vmconfig_path}" >&2
    exit 1
  }

  mv "${tmp_path}" "${vmconfig_path}"
}

customize_linux_riscv64_vmconfig_for_asterinas() {
  local vmconfig_path passthrough_devices passthrough_addresses

  vmconfig_path="$(guest_vmconfig_path)"
  if [ ! -f "${vmconfig_path}" ]; then
    echo "missing generated VM config for Asterinas customization: ${vmconfig_path}" >&2
    exit 1
  fi

  sed -i 's|^cmdline *=.*|cmdline = "earlycon=sbi console=ttyS0,115200 init=/bin/sh root=/dev/vda rw"|' \
    "${vmconfig_path}"

  passthrough_devices='passthrough_devices = [
    ["/soc/serial@10000000"],
    ["/soc/virtio_mmio@10008000"],'
  rewrite_vmconfig_array_block "${vmconfig_path}" "passthrough_devices" "${passthrough_devices}"

  passthrough_addresses='passthrough_addresses = [
    [0x1000_0000, 0x100],      # ns16550a guest console
    [0x1000_8000, 0x1000],     # virtio-mmio guest rootfs'
  rewrite_vmconfig_array_block "${vmconfig_path}" "passthrough_addresses" "${passthrough_addresses}"
}

customize_guest_assets_for_asterinas() {
  case "${GUEST}" in
    linux-riscv64)
      customize_linux_riscv64_vmconfig_for_asterinas
      ;;
  esac
}

guest_vmconfig_path() {
  generated_vmconfig_path "${GUEST}"
}

guest_rootfs_path() {
  echo "${TGOSKITS_ROOT}/os/axvisor/tmp/rootfs.img"
}

guest_qemu_args() {
  local rootfs qemu_args

  if ! guest_requires_host_rootfs_injection "${GUEST}"; then
    echo "${AXVISOR_EXTRA_QEMU_ARGS:-}"
    return
  fi

  rootfs="$(guest_rootfs_path)"
  case "${GUEST}" in
    linux-riscv64)
      qemu_args="-drive if=none,format=raw,id=axvisor_guest_rootfs,file=${rootfs} -device virtio-blk-device,drive=axvisor_guest_rootfs"
      ;;
    nimbos|nimbos-uefi|linux-x86_64-uefi)
      qemu_args="-smp 2 -device virtio-blk-pci,drive=disk0 -drive id=disk0,if=none,format=raw,file=${rootfs}"
      ;;
    *)
      echo "unsupported guest: ${GUEST}" >&2
      exit 1
      ;;
  esac

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
    qemu_args="$(guest_qemu_args)"

    if [ ! -f "${vmconfig}" ]; then
      echo "missing generated VM config: ${vmconfig}" >&2
      exit 1
    fi

    if guest_requires_host_rootfs_injection "${GUEST}"; then
      rootfs="$(guest_rootfs_path)"
      if [ ! -f "${rootfs}" ]; then
        echo "missing guest rootfs image: ${rootfs}" >&2
        exit 1
      fi
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

if [ -z "${TARGET_ARCH}" ]; then
  TARGET_ARCH="$(default_target_arch)"
fi

if [ -n "${GUEST}" ]; then
  REQUIRED_GUEST_ARCH="$(guest_target_arch "${GUEST}")"
  if [ "${TARGET_ARCH}" != "${REQUIRED_GUEST_ARCH}" ]; then
    echo "guest ${GUEST} requires --target-arch ${REQUIRED_GUEST_ARCH}; got ${TARGET_ARCH}" >&2
    exit 1
  fi
fi

if [ -n "${GUEST}" ]; then
  prepare_guest_assets
  customize_guest_assets_for_asterinas
fi

prepare_initramfs

if [ "${MODE}" != "prepare" ]; then
  run_axvisor
fi
