{ stdenvNoCC, fetchurl, pname ? "firecracker", firecrackerUrl
, firecrackerSha256, unpackArchive ? false, binaryPattern ? "firecracker-*" }:

stdenvNoCC.mkDerivation {
  inherit pname;
  version = "release";

  src = fetchurl {
    url = firecrackerUrl;
    hash = firecrackerSha256;
  };

  dontUnpack = !unpackArchive;

  installPhase = if unpackArchive then ''
    mkdir -p $out/bin
    candidate="$(find . -type f -name '${binaryPattern}' -print -quit)"
    if [ -z "$candidate" ]; then
      echo "firecracker binary matching '${binaryPattern}' not found" >&2
      exit 1
    fi
    cp "$candidate" $out/bin/firecracker
    chmod +x $out/bin/firecracker
  '' else ''
    mkdir -p $out/bin
    cp $src $out/bin/firecracker
    chmod +x $out/bin/firecracker
  '';
}
