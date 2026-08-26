{ stdenv, }:
stdenv.mkDerivation {
  pname = "virtio-net-peer";
  version = "0.1.0";
  src = builtins.path { path = ./../../src/axvisor/virtio_net_peer.c; };
  dontUnpack = true;
  buildInputs = [ stdenv.cc.libc.static ];
  buildPhase = ''
    ${stdenv.cc.targetPrefix}cc -Wall -Werror -static "$src" -o virtio_net_peer
  '';
  installPhase = ''
    mkdir -p $out/bin
    cp virtio_net_peer $out/bin/
  '';
}
