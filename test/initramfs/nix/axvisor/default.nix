{ stdenvNoCC, callPackage, }:
let
  kvmSmoke = callPackage ./kvm-smoke.nix { };
  lkvm = callPackage ./lkvm.nix { };
  lkvmRiscvPayload = callPackage ./lkvm-riscv-payload.nix { };
in stdenvNoCC.mkDerivation {
  pname = "axvisor-initramfs-tests";
  version = "0.1.0";
  dontUnpack = true;

  passthru.packages = [ kvmSmoke lkvm lkvmRiscvPayload ];

  buildCommand = ''
    mkdir -p $out/test
    cp ${kvmSmoke}/bin/kvm_smoke $out/test/kvm_smoke
    cp ${lkvm}/bin/lkvm $out/test/lkvm
    cp ${lkvmRiscvPayload}/bin/lkvm_riscv_payload $out/test/lkvm_riscv_payload
  '';
}
