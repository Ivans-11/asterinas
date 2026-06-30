{ stdenvNoCC, lib, callPackage, targetArch ? "x86_64"
, firecrackerRiscv64Url ? "", firecrackerRiscv64Sha256 ? "", }:
let
  kvmSmoke = callPackage ./kvm-smoke.nix { };
  lkvm = callPackage ./lkvm.nix { };
  firecrackerRiscvConfig = builtins.path {
    path = ./../../src/axvisor/firecracker_riscv.json;
  };
  hasFirecracker = firecrackerRiscv64Url != "" && firecrackerRiscv64Sha256 != "";
  firecracker = callPackage ./firecracker.nix {
    inherit firecrackerRiscv64Url firecrackerRiscv64Sha256;
  };
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
    ] ++ lib.optionals hasFirecracker [
      {
        package = firecracker;
        source = "${firecracker}/bin/firecracker";
        target = "firecracker";
      }
      {
        package = firecrackerRiscvConfig;
        source = firecrackerRiscvConfig;
        target = "firecracker-riscv.json";
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
