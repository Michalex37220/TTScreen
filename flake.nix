{
  description = "Digital ttscreen - environnement de développement reproductible";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (s: f nixpkgs.legacyPackages.${s});
    in {
      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.clippy
            pkgs.rustfmt
            pkgs.cargo-watch
            pkgs.pkg-config
            pkgs.nodejs_22
            pkgs.docker-compose
            pkgs.caddy
            pkgs.ffmpeg
            pkgs.sqlite
          ];
          shellHook = ''
            export TTSCREEN_ROOT="$PWD"
            echo "TTScreen dev shell prêt : ./scripts/dev.sh | ./scripts/test.sh"
          '';
        };
      });
    };
}
