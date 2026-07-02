{ stdenvNoCC, lib, callPackage, targetArch ? "x86_64"
, firecrackerX86_64Url ? "", firecrackerX86_64Sha256 ? ""
, firecrackerRiscv64Url ? "", firecrackerRiscv64Sha256 ? "", }:
let
  kvmSmoke = callPackage ./kvm-smoke.nix { };
  lkvm = callPackage ./lkvm.nix { };
  firecrackerX86Config = builtins.path {
    path = ./../../src/axvisor/firecracker_x86_64.json;
  };
  firecrackerRiscvConfig = builtins.path {
    path = ./../../src/axvisor/firecracker_riscv.json;
  };
  hasFirecrackerX86_64 =
    firecrackerX86_64Url != "" && firecrackerX86_64Sha256 != "";
  hasFirecrackerRiscv64 =
    firecrackerRiscv64Url != "" && firecrackerRiscv64Sha256 != "";
  firecrackerX86_64 = callPackage ./firecracker.nix {
    pname = "firecracker-x86_64";
    firecrackerUrl = firecrackerX86_64Url;
    firecrackerSha256 = firecrackerX86_64Sha256;
    unpackArchive = true;
    binaryPattern = "firecracker-v*-x86_64";
  };
  firecrackerRiscv64 = callPackage ./firecracker.nix {
    pname = "firecracker-riscv64";
    firecrackerUrl = firecrackerRiscv64Url;
    firecrackerSha256 = firecrackerRiscv64Sha256;
  };
  commonTestFiles = [
    {
      package = kvmSmoke;
      source = "${kvmSmoke}/bin/kvm_smoke";
      target = "kvm_smoke";
    }
  ];
  archTestFiles = {
    x86_64 = lib.optionals hasFirecrackerX86_64 [
      {
        package = firecrackerX86_64;
        source = "${firecrackerX86_64}/bin/firecracker";
        target = "firecracker";
      }
      {
        package = firecrackerX86Config;
        source = firecrackerX86Config;
        target = "firecracker-x86_64.json";
      }
    ];
    riscv64 = [
      {
        package = lkvm;
        source = "${lkvm}/bin/lkvm";
        target = "lkvm";
      }
    ] ++ lib.optionals hasFirecrackerRiscv64 [
      {
        package = firecrackerRiscv64;
        source = "${firecrackerRiscv64}/bin/firecracker";
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
