{
  description = "Walking Viewer - Sneakerweb drop reader based on PerryTS";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "sneakerweb-wrapper";
            version = "1.0.0";
            src = ./.;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ ];
            doCheck = false;
          };
        }
      );

      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            buildInputs = [
              pkgs.cargo
              pkgs.rustc
              pkgs.bun
              pkgs.pkg-config
              pkgs.clang
              pkgs.llvm
              pkgs.gtk4
              pkgs.webkitgtk_6_0
              pkgs.libshumate
              pkgs.harfbuzz
              pkgs.gdk-pixbuf
              pkgs.graphene
              pkgs.libpulseaudio
              pkgs.gst_all_1.gstreamer
              pkgs.gst_all_1.gst-plugins-base
              pkgs.libsoup_3
              pkgs.glib
              pkgs.gsettings-desktop-schemas
            ];

            shellHook = ''
              echo "Welcome to walking-viewer dev environment!"
              echo "Rust, Bun, and GUI libraries loaded."
              # Compile HTTP stubs to a static library to satisfy PerryTS stdlib dependencies
              gcc -c stubs.c -o stubs.o 2>/dev/null && ar rcs libstubs.a stubs.o 2>/dev/null
              export NIX_LDFLAGS="-L$PWD -lstubs $NIX_LDFLAGS"
              if [ -z "$XDG_DATA_DIRS" ]; then
                export XDG_DATA_DIRS="$GSETTINGS_SCHEMAS_PATH"
              else
                export XDG_DATA_DIRS="$GSETTINGS_SCHEMAS_PATH:$XDG_DATA_DIRS"
              fi
            '';
          };
        }
      );
    };
}
