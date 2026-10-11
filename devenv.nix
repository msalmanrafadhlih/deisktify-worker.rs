{ templateInputs }:
{
  pkgs,
  ...
}:
let
  system = pkgs.stdenv.hostPlatform.system;
  fenix = templateInputs.rust.inputs.fenix;
in
{
  imports = [ templateInputs.rust.devenvModules.default ];

  setupRust = {
    enable = true;
    # Target Worker. Tanpa ini `cargo check --target wasm32-unknown-unknown` gagal.
    toolchains = [ fenix.packages.${system}.targets.wasm32-unknown-unknown.stable.rust-std ];
  };

  packages = with pkgs; [
    wrangler # deploy dan `wrangler dev`
    turso-cli # `turso dev` (database lokal), `turso db ...`
    nodejs_22 # skrip fixture (scripts/gen_golden.mjs)
    binaryen # wasm-opt, dipakai worker-build
    git
  ];

  scripts = {
    # Terminal 1: database lokal di port 8081 (8080 dipakai `dx serve`).
    db-dev.exec = "turso dev --db-file local.db --port 8081";
    # Terminal 2: Worker di http://127.0.0.1:8787 (butuh crates/api/.dev.vars).
    api-dev.exec = "cd crates/api && wrangler dev";
    # Pemeriksaan yang sama dengan CI.
    check-all.exec = ''
      set -e
      cargo fmt --all -- --check
      cargo clippy --workspace --all-targets --locked -- -D warnings
      cargo test --workspace --locked
      cargo clippy -p api --target wasm32-unknown-unknown --locked -- -D warnings
    '';
  };

  enterShell = ''
    echo "tquilla-api dev shell"
    echo "  pertama kali : cargo install worker-build --version 0.8.7 --locked"
    echo "  db-dev       : database lokal (turso dev)"
    echo "  api-dev      : wrangler dev"
    echo "  check-all    : fmt + clippy + test + wasm32"
  '';
}
