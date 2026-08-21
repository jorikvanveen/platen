{
  inputs = {
    utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, utils }: utils.lib.eachDefaultSystem (system:
    let
      pkgs = nixpkgs.legacyPackages.${system};
      version = "0.0.3";
    in
    {
      devShell = pkgs.mkShell {
        buildInputs = with pkgs; [
          rustc
          cargo
          rust-analyzer
          rustfmt
          clippy
          nodejs
          yt-dlp
          pkg-config
          openssl
          sea-orm-cli
        ];
      };

      packages.platen-backend = pkgs.rustPlatform.buildRustPackage {
        pname = "platen-backend";
        inherit version;
        src = ./platen-backend;
        cargoLock = {
          lockFile = ./platen-backend/Cargo.lock;
        };
        
        nativeBuildInputs = [
          pkgs.openssl
          pkgs.pkg-config
        ];

        buildInputs = [
          pkgs.unzip
        ];
        
        PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
      };

      packages.platen-frontend = pkgs.buildNpmPackage {
        pname = "platen-frontend";
        inherit version;
        src = ./platen-frontend;
        npmDeps = pkgs.importNpmLock {
          npmRoot = ./platen-frontend;
        };
        npmConfigHook = pkgs.importNpmLock.npmConfigHook;
        installPhase = ''
          cp -r ./build $out
        '';
      };
    }
  );
}
