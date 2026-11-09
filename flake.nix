{
  description = "Musika — keyboard-driven Subsonic/Navidrome player (Rust + GPUI)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

      # App-scoped typeface: exposed to fontconfig through XDG_DATA_DIRS (by
      # the wrapper below and the dev shell), never installed system-wide.
      # Overused Grotesk stands in for the unpackaged Bricolage Grotesque and
      # is the single face for wordmark, lyrics and body (Theme.fonts).
      fonts = with pkgs; [
        overused-grotesk
      ];
      fontDataDirs = pkgs.lib.makeSearchPath "share" fonts;


      musika = pkgs.rustPlatform.buildRustPackage {
        pname = "musika";
        version = (pkgs.lib.importTOML ./Cargo.toml).package.version;

        # Keep build artefacts and local state out of the store copy.
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: _type:
            let
              base = baseNameOf (toString path);
            in
            !(builtins.elem base [
              ".git"
              ".direnv"
              "target"
              "result"
              "app.db"
              "app.db-shm"
              "app.db-wal"
            ]);
        };

        cargoLock.lockFile = ./Cargo.lock;

        # Packaging is the release profile only. Tests run in the dev shell/CI;
        # skipping the test-profile rebuild keeps nix builds fast.
        doCheck = false;
        CARGO_PROFILE_RELEASE_LTO = "thin";
        CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "4";

        nativeBuildInputs = [
          pkgs.pkg-config
          pkgs.makeWrapper
          pkgs.librsvg # rsvg-convert for the hicolor PNG sizes
          pkgs.desktop-file-utils # desktop-file-validate
        ];

        buildInputs = [
          pkgs.openssl
          pkgs.pipewire
          pkgs.ffmpeg
          pkgs.liburing
          pkgs.sqlite
          pkgs.libglvnd
          # GPUI display/font/Vulkan dependencies.
          pkgs.wayland
          pkgs.libxkbcommon
          pkgs.libx11
          pkgs.libxcursor
          pkgs.libxrandr
          pkgs.libxi
          pkgs.libxcb
          pkgs.fontconfig
          pkgs.freetype
          pkgs.vulkan-loader
        ];

        # The sandbox has no /etc/ssl; reqwest builds a client even in tests.
        SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

        # ARC-41: refuse an unbaked icon asset before compiling.
        preBuild = ''
          sh scripts/check-icons.sh
        '';

        # Wrap by hand: add the app font and certificate paths.

        postInstall = ''
          # The core daemon and its GPUI child share the runtime environment.
          # musika-ui is spawned by the daemon as a sibling binary, so it needs
          # the same wayland/vulkan/font/cert/asset wrapper as musika.
          for bin in musika musika-ui; do
            wrapProgram $out/bin/$bin \
              --prefix XDG_DATA_DIRS : ${fontDataDirs} \
              --prefix LD_LIBRARY_PATH : ${
                pkgs.lib.makeLibraryPath [
                  pkgs.wayland
                  pkgs.libxkbcommon
                  pkgs.libx11
                  pkgs.libxcursor
                  pkgs.libxrandr
                  pkgs.libxi
                  pkgs.libxcb
                  pkgs.fontconfig
                  pkgs.freetype
                  pkgs.libglvnd
                  pkgs.vulkan-loader
                ]
              } \
              --set-default SSL_CERT_FILE ${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt \
              --set-default MUSIKA_ASSET_DIR $out/share/musika
          done

          # Runtime SVG assets: icons resolve through MUSIKA_ASSET_DIR/icons,
          # not through the build directory.
          install -d $out/share/musika/icons
          install -m644 src/icons/*.svg $out/share/musika/icons/

          # Desktop entry (PKG-1): launchers, docks, MIME data dirs.
          install -Dm644 musika.desktop $out/share/applications/musika.desktop
          ${pkgs.desktop-file-utils}/bin/desktop-file-validate \
            $out/share/applications/musika.desktop

          # Application icon (PKG-1): hicolor theme, scalable + the full PNG
          # size ladder desktops ask for (16..512).
          install -Dm644 src/icons/musika.svg \
            $out/share/icons/hicolor/scalable/apps/musika.svg
          for size in 16 24 32 48 64 128 256 512; do
            install -d $out/share/icons/hicolor/''${size}x''${size}/apps
            ${pkgs.librsvg}/bin/rsvg-convert -w $size -h $size \
              src/icons/musika.svg \
              -o $out/share/icons/hicolor/''${size}x''${size}/apps/musika.png
          done
        '';

        meta = {
          description = "Keyboard-driven Subsonic/Navidrome player";
          mainProgram = "musika";
        };
      };
    in
    {
      packages.${system} = {
        default = musika;
        musika = musika;
      };

      apps.${system}.default = {
        type = "app";
        program = "${musika}/bin/musika";
      };

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        # Should be a proper way to organize this, perhaps when in build time to actually adjust
        # the shell, since it exposes all, in case which can, or cannot, be used at all.
        buildInputs = with pkgs; [
          rustc
          cargo
          rust-analyzer
          sqlite
          sqlite-analyzer
          sqls
          rustfmt
          libglvnd
          liburing
          pipewire
          ffmpeg
          openssl
          sqlx-cli
          clippy
          vulkan-tools # vulkaninfo for test::benchmark::vulkan_probe
          # GPUI (Zed) display/font/Vulkan runtime dependencies.
          wayland
          wayland-protocols
          libxkbcommon
          libx11
          libxcursor
          libxrandr
          libxi
          libxcb
          fontconfig
          freetype
          vulkan-loader
        ];

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (
          with pkgs; [
            wayland libxkbcommon libx11 libxcursor libxrandr libxi libxcb
            fontconfig freetype libglvnd vulkan-loader
          ]
        );

        shellHook = ''
          export XDG_DATA_DIRS="${fontDataDirs}''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
        '';
      };
    };
}
