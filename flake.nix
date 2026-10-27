{
  description = "Digital signage - environnement de développement reproductible";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (s: f nixpkgs.legacyPackages.${s});
    in {
      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = [
            (pkgs.python312.withPackages (ps: with ps; [
              fastapi uvicorn sqlalchemy pydantic python-multipart
              httpx pytest psycopg
            ]))
            pkgs.nodejs_22
            pkgs.docker-compose
            pkgs.caddy
            pkgs.ffmpeg
            pkgs.sqlite
          ];
          shellHook = ''
            export SIGNAGE_ROOT="$PWD"
            echo "Signage dev shell prêt : ./scripts/dev.sh | ./scripts/test.sh"
          '';
        };
      });
    };
}
