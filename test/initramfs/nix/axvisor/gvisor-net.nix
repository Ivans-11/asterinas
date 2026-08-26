{ stdenv, }:
stdenv.mkDerivation {
  pname = "gvisor-net-test";
  version = "0.1.0";
  src = builtins.path { path = ./../../src/axvisor/gvisor_net_test.c; };
  dontUnpack = true;
  buildInputs = [ stdenv.cc.libc.static ];
  buildPhase = ''
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -static "$src" -o gvisor_net_test
  '';
  installPhase = ''
    mkdir -p $out/bin
    cp gvisor_net_test $out/bin/
  '';
}
