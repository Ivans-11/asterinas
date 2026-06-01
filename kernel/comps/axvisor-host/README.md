# Asterinas Axvisor Host

This component hosts `axvisor_core` on top of the Asterinas kernel runtime.

The `tools/axvisor` helper is the single entrypoint for Asterinas-hosted
Axvisor workflows. It defaults to `x86_64`. For host-only bring-up on the
default architecture, run:

```bash
cd /home/vans/hyper/asterinas
tools/axvisor run
```

For the current x86_64 guest boot flow, run:

```bash
cd /home/vans/hyper/asterinas
tools/axvisor run --guest nimbos
```

For the current riscv64 guest flows, run:

```bash
cd /home/vans/hyper/asterinas
tools/axvisor run --arch riscv64 --guest linux
tools/axvisor run --arch riscv64 --guest arceos
```

For bounded guest verification, use `test`:

```bash
cd /home/vans/hyper/asterinas
tools/axvisor test --guest nimbos
tools/axvisor test --arch riscv64 --guest linux
tools/axvisor test --arch riscv64 --guest arceos
```

The new tooling path:

- loads host baseline config from `test-suit/axvisor/<arch>/host.toml`
  such as `["axvisor", "vmx"]` on x86_64 and `["axvisor", "sstc"]` on riscv64
- stages static case assets from `test-suit/axvisor/<arch>/<guest>/`
- downloads and caches guest images under `target/axvisor/images/`
- stages per-run VM configs under `target/axvisor/cases/`
- injects `AXVISOR_VM_CONFIGS` for `axvisor_core` build-time embedding
- points `VDSO_LIBRARY_DIR` at the local vDSO artifacts
- merges host `features` / `host_qemu_args` with case-owned `extra_features` / `extra_qemu_args`
- uses per-case static `vm.toml` instead of patching generated configs at runtime
- uses `test` to watch QEMU output, inject guest-side shell commands when needed,
  and decide pass/fail from case-owned regexes

Current validated milestone:

- `tools/axvisor run --guest nimbos` boots the current x86_64 NimbOS guest path.
- `tools/axvisor run --arch riscv64 --guest linux` reaches the guest `/bin/sh`
  prompt with passthrough `ttyS0` console and `virtio-blk` rootfs.
- `tools/axvisor run --arch riscv64 --guest arceos` boots the current RISC-V
  ArceOS guest image path.
