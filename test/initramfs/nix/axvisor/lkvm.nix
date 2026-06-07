{ stdenv, fetchFromGitHub, dtc, }:
stdenv.mkDerivation {
  pname = "lkvm";
  version = "2026-06-07";

  src = fetchFromGitHub {
    owner = "kvmtool";
    repo = "kvmtool";
    rev = "eb915c763e4b95dd4a9d32763fd449459742e053";
    hash = "sha256-MIidao1EEC/6dAM/Y5OKb0X16dzZ00rb6YfHVHa74jU=";
  };

  dontConfigure = true;

  buildPhase = ''
    runHook preBuild
    mkdir libfdt
    ln -s ${dtc}/include/libfdt.h ${dtc}/include/libfdt_env.h ${dtc}/include/fdt.h libfdt/
    ln -s ${dtc}/lib/libfdt.a libfdt/
    make ARCH=riscv RISCV_XLEN=64 CROSS_COMPILE=${stdenv.cc.targetPrefix} \
      LIBFDT_DIR=$PWD/libfdt V=1 WERROR=0 lkvm
    runHook postBuild
  '';

  installPhase = ''
    mkdir -p $out/bin
    cp lkvm $out/bin/lkvm
  '';
}
