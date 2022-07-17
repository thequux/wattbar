{
  description = "A very basic flake";

  inputs = {
    cargo2nix.url = "github:cargo2nix/cargo2nix/release-0.11.0";
    flake-utils.follows = "cargo2nix/flake-utils";
    nixpkgs.follows = "cargo2nix/nixpkgs";
  };


  outputs = inputs: with inputs; flake-utils.lib.eachDefaultSystem (system:
    let 
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ cargo2nix.overlays.default ];
      };

      overrides = [
        (pkgs.rustBuilder.rustLib.makeOverride {
          name = "smithay-client-toolkit";
          overrideAttrs = attrs: {
            buildInputs = (attrs.buildInputs or []) ++ [
              pkgs.libxkbcommon
            ];
          };
        })
      ] ++ pkgs.rustBuilder.overrides.all;

      rustPkgs = pkgs.rustBuilder.makePackageSet {
        rustVersion = "1.61.0";
        packageFun = import ./Cargo.nix;
        packageOverrides = pkgs: overrides;
      };

    in rec {
      packages = {
        wattbar = (rustPkgs.workspace.wattbar {}).bin;
        wattbarAll = rustPkgs.workspace.wattbar {};
        default = packages.wattbar;
      };
    }
  );
}
