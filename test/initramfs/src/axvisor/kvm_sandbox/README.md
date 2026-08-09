# AxVisor KVM application sandbox

This directory contains a freestanding application sandbox used by the
AxVisor control-plane tests. The common runner creates one VM and one vCPU,
loads a bounded static ELF64 application, and starts an architecture-specific
monitor. The monitor enters the application at user privilege and reports
Linux syscalls through KVM exits (`KVM_EXIT_IO` on x86_64 and `KVM_EXIT_MMIO`
on RISC-V).

`runtime.c`, `runner.c`, `elf_loader.c`, and `syscall_proxy.c` are shared by both
architectures. Register setup, page tables, privilege transitions, and KVM
exit decoding remain in `backend_*` and `monitor_*`. `sandbox_abi.h` defines the
versioned control page shared by the runner and monitors.

The loader currently accepts bounded, non-overlapping `ET_EXEC` images for the
target ISA, rejects interpreters and writable executable segments, and loads
segments at their declared guest addresses. The entry point may be any
executable address in the application window. The runner supplies a minimal
Linux initial stack (bounded command-line arguments, an empty environment,
ELF/page-size auxiliary-vector entries, and `AT_RANDOM`). The syscall proxy
supports the application I/O and process calls used by the tests, plus the
small startup subset required by a statically linked glibc program. All guest
pointers are restricted to readable or writable ELF pages, allocated heap, or
the user stack. File access is denied by default. Each readable host file
must be granted explicitly before `--`, for example:

```sh
kvm_sandbox --allow-read /data/input -- /app/program /data/input
```

Only exact path matches may be opened, all opens are read-only, and guest file
descriptors are kept in a bounded table separate from host descriptors.
The common runner bounds guest memory, KVM event count, total proxied I/O, and
wall-clock execution time. A supervisor process stops a non-cooperative vCPU
through KVM's `immediate_exit` mechanism.

The packaged test runs several external images through the same runner. The
handwritten ABI images cover separate RX and RW load segments, BSS zeroing,
heap growth, time, output, and policy-backed file input. A separately compiled
freestanding C image validates that the ELF loader, Linux initial stack, and
architecture entry stubs can also execute compiler-generated code with
arguments. A normally compiled static glibc C image additionally validates
libc startup, heap setup, random seeding, RELRO setup, standard-output metadata,
and orderly process exit without a custom entry stub. Negative cases reject a
malformed ELF, protected monitor/control
page access, an application that spins past its deadline, and file access
without an explicit grant. Together these cases cover structured faults,
cleanup, VM recreation after failure, forced exit, and default-deny input.
Full-system tests are:

```sh
tools/axvisor test --arch x86_64 --mode control --case sandbox
tools/axvisor test --arch riscv64 --mode control --case sandbox
```

The current scope is static, single-threaded applications. It does not yet
provide writable files, general mappings or protection changes, threads,
signals, or a broad Linux syscall surface.
`DESIGN.md` tracks that boundary and the remaining hardening work.
