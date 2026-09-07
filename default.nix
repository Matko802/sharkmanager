# Classic (non-flake) Nix package for sharkmanager.
#
# Lets you add SharkManager to your existing channel-based NixOS config
# WITHOUT converting your whole system to flakes, e.g. in configuration.nix:
#
#   environment.systemPackages = [
#     (pkgs.callPackage /mnt/ssd/My-Files/Projects/sharkmanager/default.nix {})
#   ];
#
# This builds with YOUR system's nixpkgs, so there is no version drift.

{ pkgs ? import <nixpkgs> {} }:

let
  # Don't ship the Rust build cache / VCS into the Nix derivation.
  src = pkgs.lib.cleanSourceWith {
    src = ./.;
    filter = path: _type:
      !(pkgs.lib.strings.hasInfix "/target/" path)
      && !(pkgs.lib.strings.hasInfix "/.git/" path);
  };
in

pkgs.rustPlatform.buildRustPackage {
  pname = "sharkmanager";
  version = "0.1.0";
  inherit src;

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = with pkgs; [ pkg-config pkgs.wrapGAppsHook4 ];
  buildInputs = with pkgs; [
    gtk4 glib cairo pango gdk-pixbuf graphene gobject-introspection libadwaita dbus
  ];

  # wrapGAppsHook sets the proper env so GTK4 themes/icons work.
  meta = with pkgs.lib; {
    description = "Thunar-like file manager built with Rust + GTK4";
    license = licenses.gpl3Plus;
    platforms = platforms.linux;
    mainProgram = "sharkmanager";
  };
}
