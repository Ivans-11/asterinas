{ stdenvNoCC, fetchurl, runscUrl, runscSha256 }:

stdenvNoCC.mkDerivation {
  pname = "runsc";
  version = "20260727.0";

  src = fetchurl {
    url = runscUrl;
    hash = runscSha256;
  };

  dontUnpack = true;

  installPhase = ''
    mkdir -p $out/bin
    cp $src $out/bin/runsc
    chmod +x $out/bin/runsc
  '';
}
