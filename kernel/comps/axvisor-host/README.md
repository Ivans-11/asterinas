# Asterinas Axvisor Host

This component hosts `axvisor_core` on top of the Asterinas kernel runtime.

For the current x86_64 probe flow, prepare one static guest config and run the
host with:

```bash
cd /home/vans/hyper/asterinas
tools/axvisor_guest_probe.sh run --guest nimbos
```

The helper script:

- prepares guest assets through `tgoskits/os/axvisor/scripts/setup_qemu.sh`
- injects `AXVISOR_VM_CONFIGS` for `axvisor_core` build-time embedding
- points `VDSO_LIBRARY_DIR` at the local vDSO artifacts
- appends the guest rootfs disk and forces `-smp 2` so Axvisor can pin the
  guest vCPU to host CPU 1

Current validated milestone:

- `tools/axvisor_guest_probe.sh run --guest nimbos` on x86_64 KVM reaches the
  NimbOS user shell on top of the Asterinas-hosted Axvisor path.
