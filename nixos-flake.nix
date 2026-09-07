# SharkManager — system flake template
#
# HOW TO USE (as root):
#   1. cp nixos-flake.nix /etc/nixos/flake.nix
#   2. nixos-rebuild switch --flake /etc/nixos#nixos
#
# This keeps your existing configuration.nix / hardware-configuration.nix and
# just adds sharkmanager to environment.systemPackages.
#
# NOTE: this pins nixpkgs to `nixos-unstable`. If you want to match your
# current 26.11 system exactly, change the nixpkgs url to:
#   nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.11";

{
  description = "NixOS configuration for nixos (with SharkManager 🦈)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Local sharkmanager project (its own flake.nix builds the Rust app)
    sharkmanager = {
      url = "path:/mnt/ssd/My-Files/Projects/sharkmanager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, sharkmanager, ... }:
    let
      system = "x86_64-linux";
    in {
      nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
        inherit system;
        # make the sharkmanager flake input available to our module
        specialArgs = { inherit sharkmanager; };
        modules = [
          ./configuration.nix

          ({ config, pkgs, sharkmanager, ... }: {
            environment.systemPackages = [
              sharkmanager.packages.${pkgs.system}.default
            ];
          })
        ];
      };
    };
}
