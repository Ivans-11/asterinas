{ target ? "x86_64", enableBenchmarkTest ? false, enableConformanceTest ? false
, enableRegressionTest ? false, conformanceTestSuite ? "ltp"
, conformanceTestWorkDir ? "/tmp", regressionTestPlatform ? "asterinas"
, dnsServer ? "none", smp ? 1, initramfsCompressed ? true
, firecrackerX86_64Url ?
  "https://github.com/firecracker-microvm/firecracker/releases/download/v1.16.0/firecracker-v1.16.0-x86_64.tgz"
, firecrackerX86_64Sha256 ?
  "sha256-vQTiaVLU4VgIV3jGIwoLOD0mGcMZGC4n6qnWGiEuktY="
, firecrackerRiscv64Url ?
  "https://github.com/Ivans-11/firecracker/releases/download/firecracker-riscv64-v0.1.1/firecracker-riscv64gc-unknown-linux-musl"
, firecrackerRiscv64Sha256 ? "sha256-j9/JbST84tbiCCgyTZkl5DUgPawEo07SV5V9jX6Qeao="
, runscUrl ?
  "https://storage.googleapis.com/gvisor/releases/release/20260727.0/x86_64/runsc"
, runscSha256 ? "sha256-bsRoCKIslLfqaN2VIegxtExp4NMmeizIYvmmrikM7pE="
, axvisorTestFiles ? "", }:
let
  crossSystem.config = if target == "x86_64" then
    "x86_64-unknown-linux-gnu"
  else if target == "riscv64" then
    "riscv64-unknown-linux-gnu"
  else
    throw "Target arch ${target} not yet supported.";

  # Pinned nixpkgs (nix version: 2.29.1, channel: nixos-25.05, release date: 2025-07-01)
  nixpkgs = fetchTarball {
    url =
      "https://github.com/NixOS/nixpkgs/archive/c0bebd16e69e631ac6e52d6eb439daba28ac50cd.tar.gz";
    sha256 = "1fbhkqm8cnsxszw4d4g0402vwsi75yazxkpfx3rdvln4n6s68saf";
  };
  pkgs = import nixpkgs {
    config = { };
    overlays = [ ];
    inherit crossSystem;
  };
  lib = pkgs.lib;
in rec {
  # Packages needed by initramfs
  busybox = pkgs.busybox;
  benchmark = pkgs.callPackage ./benchmark { };
  conformance = pkgs.callPackage ./conformance {
    inherit smp;
    testSuite = conformanceTestSuite;
    workDir = conformanceTestWorkDir;
  };
  regression =
    pkgs.callPackage ./regression { testPlatform = regressionTestPlatform; };
  axvisorTests = pkgs.callPackage ./axvisor {
    targetArch = target;
    testFiles = lib.filter (file: file != "") (lib.splitString "," axvisorTestFiles);
    inherit firecrackerX86_64Url firecrackerX86_64Sha256
      firecrackerRiscv64Url firecrackerRiscv64Sha256;
    inherit runscUrl runscSha256;
  };

  initramfs = pkgs.callPackage ./initramfs.nix {
    inherit busybox axvisorTests;
    benchmark = if enableBenchmarkTest then benchmark else null;
    conformance = if enableConformanceTest then conformance else null;
    regression = if enableRegressionTest then regression else null;
    dnsServer = dnsServer;
  };
  initramfs-image = pkgs.callPackage ./initramfs-image.nix {
    inherit initramfs;
    compressed = initramfsCompressed;
  };

  # Packages needed by host
  apacheHttpd = pkgs.apacheHttpd;
  iperf3 = pkgs.iperf3;
  libmemcached = pkgs.libmemcached.overrideAttrs (_: {
    configureFlags = [ "--enable-memaslap" ];
    LDFLAGS = "-lpthread";
    CPPFLAGS = "-fcommon -fpermissive";
  });
  lmbench = pkgs.callPackage ./benchmark/lmbench.nix { };
  redis = (pkgs.redis.overrideAttrs (_: { doCheck = false; })).override {
    withSystemd = false;
  };
}
