{ stdenv }:
let
  sources =
    if stdenv.hostPlatform.isRiscV then
      ''"$src/runner.c" "$src/runtime.c" "$src/elf_loader.c" "$src/syscall_proxy.c" "$src/backend_riscv64.c" "$src/monitor_riscv64.S" "$src/runner_entry_riscv64.S"''
    else
      ''"$src/runner.c" "$src/runtime.c" "$src/elf_loader.c" "$src/syscall_proxy.c" "$src/backend_x86_64.c" "$src/monitor_x86_64.S" "$src/runner_entry_x86_64.S"'';
  app_source = if stdenv.hostPlatform.isRiscV then
    "$src/app_riscv64.S"
  else
    "$src/app_x86_64.S";
  app_entry = if stdenv.hostPlatform.isRiscV then
    "$src/app_entry_riscv64.S"
  else
    "$src/app_entry_x86_64.S";
in
stdenv.mkDerivation {
  pname = "kvm-sandbox";
  version = "0.1.0";
  src = builtins.path { path = ./../../src/axvisor/kvm_sandbox; };
  dontUnpack = true;
  buildInputs = [ stdenv.cc.libc.static ];
  buildPhase = ''
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-builtin \
      -fno-stack-protector -nostdlib -static -no-pie ${sources} \
      -o kvm_sandbox
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-builtin \
      -fno-stack-protector -nostdlib -static -no-pie -Wl,-T,$src/app.ld \
      ${app_source} -o kvm_sandbox_app
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-builtin \
      -fno-stack-protector -nostdlib -static -no-pie -DSANDBOX_APP_ALT \
      -Wl,-T,$src/app.ld ${app_source} -o kvm_sandbox_app_alt
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-builtin \
      -fno-stack-protector -nostdlib -static -no-pie -DSANDBOX_APP_MONITOR_ATTACK \
      -Wl,-T,$src/app.ld ${app_source} -o kvm_sandbox_monitor_attack
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-builtin \
      -fno-stack-protector -nostdlib -static -no-pie -DSANDBOX_APP_CONTROL_ATTACK \
      -Wl,-T,$src/app.ld ${app_source} -o kvm_sandbox_control_attack
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-builtin \
      -fno-stack-protector -nostdlib -static -no-pie -DSANDBOX_APP_SPIN \
      -Wl,-T,$src/app.ld ${app_source} -o kvm_sandbox_spin
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-builtin \
      -fno-stack-protector -nostdlib -static -no-pie -DSANDBOX_APP_FILE \
      -Wl,-T,$src/app.ld ${app_source} -o kvm_sandbox_file_app
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-builtin \
      -fno-stack-protector -nostdlib -static -no-pie -Wl,-T,$src/app.ld \
      $src/app_c.c ${app_entry} -o kvm_sandbox_c_app
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -static -no-pie \
      -Wl,-Ttext-segment=0x10000 $src/app_libc.c -o kvm_sandbox_libc_app
  '';
  installPhase = ''
    mkdir -p $out/bin
    cp kvm_sandbox $out/bin/
    cp kvm_sandbox_app $out/bin/
    cp kvm_sandbox_app_alt $out/bin/
    cp kvm_sandbox_monitor_attack $out/bin/
    cp kvm_sandbox_control_attack $out/bin/
    cp kvm_sandbox_spin $out/bin/
    cp kvm_sandbox_file_app $out/bin/
    cp kvm_sandbox_c_app $out/bin/
    cp kvm_sandbox_libc_app $out/bin/
    cp $src/data.fixture $out/bin/kvm_sandbox_data
    cp $src/invalid_elf.fixture $out/bin/kvm_sandbox_invalid_elf
  '';
}
