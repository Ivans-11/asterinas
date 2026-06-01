# Asterinas Axvisor Host

This component hosts `axvisor_core` on top of the Asterinas kernel runtime.

The unified helper script defaults to `x86_64`. For host-only bring-up on the
default architecture, run:

```bash
cd /home/vans/hyper/asterinas
tools/axvisor_run.sh run
```

For the current x86_64 guest boot flow, run:

```bash
cd /home/vans/hyper/asterinas
tools/axvisor_run.sh run --guest nimbos
```

For the current riscv64 Linux guest boot flow, run:

```bash
cd /home/vans/hyper/asterinas
tools/axvisor_run.sh run --target-arch riscv64 --guest linux-riscv64
```

The unified helper script:

- prepares guest assets through `tgoskits/os/axvisor/scripts/setup_qemu.sh`
- injects `AXVISOR_VM_CONFIGS` for `axvisor_core` build-time embedding
- points `VDSO_LIBRARY_DIR` at the local vDSO artifacts
- appends the guest rootfs disk when the selected guest expects host-side disk
  injection
- forces `-smp 2` for the current x86_64 guest path so Axvisor can pin the
  guest vCPU to host CPU 1

For other host architectures, add `--target-arch` explicitly:

```bash
cd /home/vans/hyper/asterinas
tools/axvisor_run.sh run --target-arch riscv64
```

Current validated milestone:

- `tools/axvisor_run.sh run --guest nimbos` on x86_64 KVM reaches the
  NimbOS user shell on top of the Asterinas-hosted Axvisor path.
- `tools/axvisor_run.sh run --target-arch riscv64 --guest linux-riscv64`
  reaches the guest `/bin/sh` prompt with passthrough `ttyS0` console and
  `virtio-blk` rootfs on top of the Asterinas-hosted Axvisor path.
