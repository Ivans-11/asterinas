{ stdenv, }:
stdenv.mkDerivation {
  pname = "kvm-smoke";
  version = "0.1.0";
  src = builtins.path { path = ./../src/kvm_smoke.c; };
  dontUnpack = true;
  buildPhase = ''
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -ffreestanding -fno-stack-protector -nostdlib -static "$src" -o kvm_smoke
  '';
  installPhase = ''
    mkdir -p $out/bin
    cp kvm_smoke $out/bin/
  '';
}
