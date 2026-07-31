{ stdenv }:
let
  sources =
    if stdenv.hostPlatform.isRiscV then
      ''"$src/main_riscv64.c" "$src/guest_riscv64.S"''
    else
      ''"$src/main_x86_64.c" "$src/guest_x86_64.S"'';
in
stdenv.mkDerivation {
  pname = "kvm-sandbox";
  version = "0.1.0";
  src = builtins.path { path = ./../../src/axvisor/kvm_sandbox; };
  dontUnpack = true;
  buildPhase = ''
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-builtin \
      -fno-stack-protector -nostdlib -static -no-pie ${sources} \
      -o kvm_sandbox
  '';
  installPhase = ''
    mkdir -p $out/bin
    cp kvm_sandbox $out/bin/
  '';
}
