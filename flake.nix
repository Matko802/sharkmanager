{
  description = "SharkManager — Thunar-like GTK4 file manager in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [];
        };
      in {
        devShells.default = pkgs.mkShell {
          name = "sharkmanager";
          nativeBuildInputs = with pkgs; [
            rustToolchain
            pkg-config
            wrapGAppsHook4
            desktop-file-utils
          ];
          buildInputs = with pkgs; [
            gtk4
            glib
            cairo
            pango
            gdk-pixbuf
            graphene
            gobject-introspection
            libadwaita
            gtksourceview5
            hicolor-icon-theme
            dbus
            ffmpegthumbnailer
          ];
          shellHook = ''
            export XDG_DATA_DIRS=$XDG_DATA_DIRS:${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}
          '';
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "sharkmanager";
          version = "0.1.0";
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: _type:
              !(pkgs.lib.strings.hasInfix "/target/" path)
              && !(pkgs.lib.strings.hasInfix "/.git/" path);
          };
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook4 ];
          buildInputs = with pkgs; [
            gtk4 glib cairo pango gdk-pixbuf graphene gobject-introspection libadwaita dbus
          ];
          preFixup = ''
            gappsWrapperArgs+=(--prefix PATH : ${pkgs.ffmpegthumbnailer}/bin)
          '';
          postInstall = ''
            install -Dm644 ${./data/sharkmanager.desktop} $out/share/applications/sharkmanager.desktop
          '';
        };

        apps.default = flake-utils.lib.mkApp { drv = self.packages.${system}.default; };
      })
  // {
    # Overlay so downstream flakes (e.g. a system flake) can do:
    #   nixpkgs.overlays = [ sharkmanager.overlays.default ];
    # and then refer to `pkgs.sharkmanager`.
    overlays.default = final: prev: {
      sharkmanager = final.rustPlatform.buildRustPackage {
        pname = "sharkmanager";
        version = "0.1.0";
        src = final.lib.cleanSourceWith {
          src = ./.;
          filter = path: _type:
            !(final.lib.strings.hasInfix "/target/" path)
            && !(final.lib.strings.hasInfix "/.git/" path);
        };
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = with final; [ pkg-config wrapGAppsHook4 ];
        buildInputs = with final; [
          gtk4 glib cairo pango gdk-pixbuf graphene gobject-introspection libadwaita dbus
        ];
        preFixup = ''
          gappsWrapperArgs+=(--prefix PATH : ${final.ffmpegthumbnailer}/bin)
        '';
        postInstall = ''
          install -Dm644 ${./data/sharkmanager.desktop} $out/share/applications/sharkmanager.desktop
        '';
      };
    };
  };
}
