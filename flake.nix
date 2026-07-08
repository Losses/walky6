{
  description = "walky6 - A tiny sneakerweb browser";

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

          sneakerwebSrc = builtins.fetchGit {
            url = "https://codeberg.org/worm-blossom/sneakerweb";
            ref = "main";
            rev = "888cf132207a2bf0622a5633a2d347e9e910538c";
          };

          vendorSneakerweb = pkgs.stdenv.mkDerivation {
            name = "sneakerweb-vendor";
            src = sneakerwebSrc;
            patches = [ ./patches/sneakerweb.patch ];
            installPhase = ''
              mkdir -p $out/sneakerweb
              cp -r . $out/sneakerweb/
            '';
          };

          babRsSrc = builtins.fetchGit {
            url = "https://codeberg.org/worm-blossom/bab_rs";
            ref = "main";
            rev = "2dd7466083424eccdecc1c2f43a36fef7acc8a83";
          };

          vendorBabRs = pkgs.stdenv.mkDerivation {
            name = "bab_rs-vendor";
            src = babRsSrc;
            patches = [ ./patches/bab_rs.patch ];
            installPhase = ''
              mkdir -p $out/bab_rs
              cp -r . $out/bab_rs/
            '';
          };
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "walky6";
            version = "1.0.0";
            src = pkgs.lib.cleanSourceWith {
              src = ./.;
              filter = path: type:
                let
                  relPath = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
                in
                  pkgs.lib.cleanSourceFilter path type ||
                  pkgs.lib.hasPrefix "walky6/" relPath ||
                  relPath == "walky6";
            };
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [
              pkgs.dbus
              pkgs.gtk3
              pkgs.webkitgtk_4_1
              pkgs.libsoup_3
              pkgs.glib
              pkgs.cairo
              pkgs.pango
              pkgs.harfbuzz
              pkgs.gdk-pixbuf
              pkgs.graphene
              pkgs.libpulseaudio
              pkgs.gst_all_1.gstreamer
              pkgs.gst_all_1.gst-plugins-base
              pkgs.gsettings-desktop-schemas
            ];
            preBuild = ''
              echo "Setting up vendored sneakerweb..."
              mkdir -p vendor
              cp -r ${vendorSneakerweb}/sneakerweb vendor/sneakerweb
              echo "Setting up vendored bab_rs..."
              cp -r ${vendorBabRs}/bab_rs vendor/bab_rs
            '';
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
              pkgs.dbus
              pkgs.cargo
              pkgs.rustc
              pkgs.bun
              pkgs.pkg-config
              pkgs.clang
              pkgs.llvm
              pkgs.gtk3
              pkgs.webkitgtk_4_1
              pkgs.libsoup_3
              pkgs.glib
              pkgs.cairo
              pkgs.pango
              pkgs.harfbuzz
              pkgs.gdk-pixbuf
              pkgs.graphene
              pkgs.libpulseaudio
              pkgs.gst_all_1.gstreamer
              pkgs.gst_all_1.gst-plugins-base
              pkgs.gsettings-desktop-schemas
            ];

            shellHook = ''
              echo "Welcome to walky6 dev environment!"
              echo "Rust, Bun, and Tauri GUI libraries loaded."
              echo ""
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
