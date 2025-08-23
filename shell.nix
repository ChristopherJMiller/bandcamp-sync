{ pkgs ? import <nixpkgs> {} }:

let
  flake = (builtins.getFlake (toString ./.));
  devShell = flake.devShells.${pkgs.system}.default;
in
pkgs.mkShell {
  buildInputs = devShell.buildInputs or [];
}