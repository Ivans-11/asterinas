{ stdenvNoCC, lib, callPackage, targetArch ? "x86_64", }:
let
  kvmSmoke = callPackage ./kvm-smoke.nix { };
  lkvm = callPackage ./lkvm.nix { };
  lkvmRiscvPayload = callPackage ./lkvm-riscv-payload.nix { };
  commonTestFiles = [
    {
      package = kvmSmoke;
      source = "${kvmSmoke}/bin/kvm_smoke";
      target = "kvm_smoke";
    }
  ];
  archTestFiles = {
    riscv64 = [
      {
        package = lkvm;
        source = "${lkvm}/bin/lkvm";
        target = "lkvm";
      }
      {
        package = lkvmRiscvPayload;
        source = "${lkvmRiscvPayload}/bin/lkvm_riscv_payload";
        target = "lkvm_riscv_payload";
      }
    ];
  };
  selectedTestFiles =
    commonTestFiles ++ lib.attrByPath [ targetArch ] [ ] archTestFiles;
  copyTestFiles = lib.concatMapStringsSep "\n" (file:
    "cp ${file.source} $out/test/${file.target}") selectedTestFiles;
in stdenvNoCC.mkDerivation {
  pname = "axvisor-initramfs-tests";
  version = "0.1.0";
  dontUnpack = true;

  passthru.packages = map (file: file.package) selectedTestFiles;

  buildCommand = ''
    mkdir -p $out/test
    ${copyTestFiles}
  '';
}
