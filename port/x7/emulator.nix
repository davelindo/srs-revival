# The pinned portable Renode release needs the host's ELF loader and native
# libraries. The minimal Nix Docker image intentionally has no /lib64 loader.
{ pkgs, defaultShell }:
pkgs.mkShell {
  inputsFrom = [ defaultShell ];
  packages = [
    pkgs.patchelf
    pkgs.openssh
  ];
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
    pkgs.stdenv.cc.cc.lib
    pkgs.zlib
    pkgs.icu
    pkgs.openssl
    pkgs.libunwind
  ];
  RENODE_ELF_LOADER = "${pkgs.glibc}/lib/ld-linux-x86-64.so.2";
}
