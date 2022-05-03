{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    libxkbcommon
    cargo
    rust-analyzer
    rustc
    rustfmt
    cargo-fmt
    # keep this line if you use bash
    bashInteractive
  ];

  nativeBuildInputs = with pkgs; [ 
    pkg-config
  ];
}
