{ stdenv, }:
stdenv.mkDerivation {
  pname = "lkvm-riscv-payload";
  version = "0.1.0";
  src = builtins.path { path = ./../../src/axvisor/lkvm_riscv_payload.S; };
  dontUnpack = true;
  buildPhase = ''
    ${stdenv.cc.targetPrefix}cc -nostdlib -static -Wl,-Ttext=0x80200000 "$src" -o lkvm_riscv_payload.elf
    ${stdenv.cc.targetPrefix}objcopy -O binary lkvm_riscv_payload.elf lkvm_riscv_payload
  '';
  installPhase = ''
    mkdir -p $out/bin
    cp lkvm_riscv_payload $out/bin/
  '';
}
