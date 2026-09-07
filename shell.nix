{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  name = "sharkmanager";
  nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook4 desktop-file-utils rustc cargo clippy rustfmt ];
  buildInputs = with pkgs; [
    gtk4 glib cairo pango gdk-pixbuf graphene gobject-introspection libadwaita
    gsettings-desktop-schemas hicolor-icon-theme
  ];
  shellHook = ''
    export XDG_DATA_DIRS=$XDG_DATA_DIRS:${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}
    echo "🦈 SharkManager dev shell — run: cargo run"
  '';
}
