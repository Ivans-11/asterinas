{ stdenvNoCC, lib, callPackage, qemu, dtc, targetArch ? "x86_64"
, firecrackerX86_64Url ? "", firecrackerX86_64Sha256 ? ""
, firecrackerRiscv64Url ? "", firecrackerRiscv64Sha256 ? ""
, testFiles ? [ ], }:
let
  hasTest = name: lib.elem name testFiles;
  supportedTestFiles = if targetArch == "x86_64" then
    [ "kvm_smoke" "kvm_sandbox" "firecracker" "qemu" ]
  else if targetArch == "riscv64" then
    [ "kvm_smoke" "kvm_sandbox" "firecracker" "lkvm" "qemu" ]
  else
    [ ];
  unknownTestFiles = lib.filter (name: !(lib.elem name supportedTestFiles)) testFiles;
  _validated = lib.assertMsg (unknownTestFiles == [ ])
    "unknown Axvisor test file(s): ${lib.concatStringsSep ", " unknownTestFiles}";
  kvmSmoke = callPackage ./kvm-smoke.nix { };
  kvmSandbox = callPackage ./kvm-sandbox.nix { };
  lkvm = callPackage ./lkvm.nix { };
  firecrackerX86Config = builtins.path {
    path = ./../../src/axvisor/firecracker_x86_64.json;
  };
  firecrackerRiscvConfig = builtins.path {
    path = ./../../src/axvisor/firecracker_riscv.json;
  };
  hasFirecrackerX86_64 =
    hasTest "firecracker" && firecrackerX86_64Url != "" && firecrackerX86_64Sha256 != "";
  hasFirecrackerRiscv64 =
    hasTest "firecracker" && firecrackerRiscv64Url != "" && firecrackerRiscv64Sha256 != "";
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
  qemuSystemArch = if targetArch == "x86_64" then
    "x86_64"
  else if targetArch == "riscv64" then
    "riscv64"
  else
    throw "QEMU KVM test is not supported for ${targetArch}";
  qemuKvmBase = qemu.override {
    hostCpuOnly = true;
    hostCpuTargets = [ "${qemuSystemArch}-softmmu" ];
    minimal = true;
    pluginsSupport = false;
    uringSupport = false;
  };
  qemuKvm = qemuKvmBase.overrideAttrs (old: {
    buildInputs = old.buildInputs ++ [ dtc ];
    configureFlags = old.configureFlags ++ [
      "--disable-curl"
      "--disable-gnutls"
      "--disable-linux-aio"
      "--disable-linux-io-uring"
    ];
  });
  qemuTestFile = {
    package = qemuKvm;
    source = "${qemuKvm}/bin/qemu-system-${qemuSystemArch}";
    target = "qemu-system-${qemuSystemArch}";
  };
  qemuX86FirmwareNames =
    [
      "bios-256k.bin"
      "bios-microvm.bin"
      "linuxboot_dma.bin"
      "linuxboot.bin"
      "kvmvapic.bin"
    ];
  qemuX86Firmware = stdenvNoCC.mkDerivation {
    pname = "qemu-x86-test-firmware";
    inherit (qemuKvm) version;
    src = qemuKvm.src;
    dontConfigure = true;
    dontBuild = true;
    installPhase = ''
      mkdir -p $out
      for firmware in ${lib.concatStringsSep " " qemuX86FirmwareNames}; do
        cp pc-bios/$firmware $out/$firmware
      done
    '';
  };
  qemuTestFiles = [ qemuTestFile ] ++ lib.optionals (targetArch == "x86_64")
    (map (firmware: {
      package = qemuX86Firmware;
      source = "${qemuX86Firmware}/${firmware}";
      target = firmware;
    }) qemuX86FirmwareNames);
  commonTestFiles = lib.optionals (hasTest "kvm_smoke") [
    {
      package = kvmSmoke;
      source = "${kvmSmoke}/bin/kvm_smoke";
      target = "kvm_smoke";
    }
  ];
  archTestFiles = {
    x86_64 = lib.optionals (hasTest "kvm_sandbox") [
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox";
        target = "kvm_sandbox";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_app";
        target = "kvm_sandbox_app";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_app_alt";
        target = "kvm_sandbox_app_alt";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_invalid_elf";
        target = "kvm_sandbox_invalid_elf";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_monitor_attack";
        target = "kvm_sandbox_monitor_attack";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_control_attack";
        target = "kvm_sandbox_control_attack";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_spin";
        target = "kvm_sandbox_spin";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_file_app";
        target = "kvm_sandbox_file_app";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_c_app";
        target = "kvm_sandbox_c_app";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_libc_app";
        target = "kvm_sandbox_libc_app";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_data";
        target = "kvm_sandbox_data";
      }
    ] ++ lib.optionals (lib.elem "qemu" testFiles) qemuTestFiles
      ++ lib.optionals hasFirecrackerX86_64 [
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
    riscv64 = lib.optionals (hasTest "kvm_sandbox") [
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox";
        target = "kvm_sandbox";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_app";
        target = "kvm_sandbox_app";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_app_alt";
        target = "kvm_sandbox_app_alt";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_invalid_elf";
        target = "kvm_sandbox_invalid_elf";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_monitor_attack";
        target = "kvm_sandbox_monitor_attack";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_control_attack";
        target = "kvm_sandbox_control_attack";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_spin";
        target = "kvm_sandbox_spin";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_file_app";
        target = "kvm_sandbox_file_app";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_c_app";
        target = "kvm_sandbox_c_app";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_libc_app";
        target = "kvm_sandbox_libc_app";
      }
      {
        package = kvmSandbox;
        source = "${kvmSandbox}/bin/kvm_sandbox_data";
        target = "kvm_sandbox_data";
      }
    ] ++ lib.optionals (hasTest "qemu") qemuTestFiles
      ++ lib.optionals (hasTest "lkvm") [
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
in assert _validated; stdenvNoCC.mkDerivation {
  pname = "axvisor-initramfs-tests";
  version = "0.1.0";
  dontUnpack = true;

  passthru.packages = map (file: file.package) selectedTestFiles;

  buildCommand = ''
    mkdir -p $out/test
    ${copyTestFiles}
  '';
}
