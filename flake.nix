{
  description = "tquilla-api: Axum di Cloudflare Workers + Turso (dev shell)";

  inputs = {
    systems.url = "github:nix-systems/default";

    rust.url = "github:msalmanrafadhlih/nixos-development-templates/main?dir=rust";

    flake-utils = {
      follows = "rust/flake-utils";
      inputs.systems.follows = "systems";
    };

    nixpkgs.follows = "rust/nixpkgs";
    devenv.follows = "rust/devenv";
  };

  outputs =
    { nixpkgs, flake-utils, ... }@inputs:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
        };
      in
      {
        devShells.default = inputs.devenv.lib.mkShell {
          inherit inputs pkgs;
          modules = [ (import ./devenv.nix { templateInputs = inputs; }) ];
        };
      }
    );
}
