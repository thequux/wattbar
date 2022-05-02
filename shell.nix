{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    libxkbcommon
    # keep this line if you use bash
    bashInteractive
  ];

  nativeBuildInputs = with pkgs; [ 
    pkg-config
  ];
}
