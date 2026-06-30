{ stdenvNoCC, fetchurl, firecrackerRiscv64Url, firecrackerRiscv64Sha256, }:

stdenvNoCC.mkDerivation {
  pname = "firecracker-riscv64";
  version = "release";

  src = fetchurl {
    url = firecrackerRiscv64Url;
    hash = firecrackerRiscv64Sha256;
  };

  dontUnpack = true;

  installPhase = ''
    mkdir -p $out/bin
    cp $src $out/bin/firecracker
    chmod +x $out/bin/firecracker
  '';
}
