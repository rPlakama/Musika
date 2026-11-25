{
  description = "Musika — keyboard-driven Subsonic/Navidrome player (Rust + GPUI)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    kache = {
      url = "github:kunobi-ninja/kache/stable";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      kache,
      self,
      nixpkgs,
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

      # Skip kache's upstream checkPhase: its sandboxed rustdoc unit test
      # flakes under Nix builds while the release binary builds cleanly.
      kachePkg = kache.packages.${system}.default.overrideAttrs (_: {
        doCheck = false;
      });

      # App-scoped typeface: exposed to fontconfig through XDG_DATA_DIRS (by
      # the wrapper below and the dev shell), never installed system-wide.
      # Overused Grotesk stands in for the unpackaged Bricolage Grotesque and
      # is the single face for wordmark, lyrics and body (Theme.fonts).
      fonts = with pkgs; [
        overused-grotesk
      ];
      fontDataDirs = pkgs.lib.makeSearchPath "share" fonts;

      # Headless FFmpeg inherited by our stripped audio-only MPV runtime.
      ffmpegAudio = pkgs.ffmpeg-headless;

      # Stripped audio-only MPV: keeps PipeWire + ALSA output backends and
      # FFmpeg lavfi audio filters (Equalizer APO, ReplayGain, gapless) while
      # dropping all video/GUI/scripting/disc/archive dependencies.
      mpvAudio =
        (pkgs.mpv-unwrapped.override {
          ffmpeg = ffmpegAudio;
          pipewireSupport = true;
          alsaSupport = true;
          pulseSupport = false;
          jackaudioSupport = false;
          openalSupport = false;
          waylandSupport = false;
          x11Support = false;
          drmSupport = false;
          vulkanSupport = false;
          vaapiSupport = false;
          vdpauSupport = false;
          sdl2Support = false;
          sixelSupport = false;
          cacaSupport = false;
          cmsSupport = false;
          archiveSupport = false;
          bluraySupport = false;
          cddaSupport = false;
          dvbinSupport = false;
          dvdnavSupport = false;
          javascriptSupport = false;
          rubberbandSupport = false;
          vapoursynthSupport = false;
          zimgSupport = false;
        }).overrideAttrs
          (old: {
            mesonFlags = (old.mesonFlags or [ ]) ++ [
              "-Dlua=disabled"
              "-Dlibmpv=false"
              "-Dcplayer=true"
            ];
          });

      musika = pkgs.rustPlatform.buildRustPackage {
        pname = "musika";
        version = (pkgs.lib.importTOML ./Cargo.toml).package.version;

        # Whitelist the inputs the build actually reads. A denylist leaked
        # third-party checkouts (DankMaterialShell/, noctalia/), local runtime
        # state (*.db, .env) and editor leftovers into the store copy on every
        # build; only Rust sources, manifests, migrations, vendored patches and
        # the packaging assets are kept.
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            let
              rel = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
              # Only files the build actually reads belong here: adding
              # README/LICENSE would invalidate the source hash on every doc
              # edit and force a full release rebuild.
              file = builtins.elem rel [
                "Cargo.toml"
                "Cargo.lock"
                "build.rs"
                "clippy.toml"
                "musika.desktop"
              ];
              tree = [
                "src"
                "examples"
                "scripts"
                "vendor"
                "migrations"
                "migrations_artist_user"
                "migrations_colors"
                "migrations_covers"
                "migrations_download"
                "migrations_library"
                "migrations_lyrics"
                "migrations_media"
                "migrations_playlist"
                "migrations_search"
              ];
              inside = name: rel == name || pkgs.lib.hasPrefix (name + "/") rel;
              keep = builtins.any inside tree;
            in
            (type == "directory" && keep) || (type == "regular" && (file || keep));
        };

        cargoLock.lockFile = ./Cargo.lock;

        # Packaging is the release profile only. Tests run in the dev shell/CI;
        # skipping the test-profile rebuild keeps nix builds fast.
        #
        # The shipped core is a long-lived process: fat LTO / one codegen unit
        # is the repo's release profile and produces a much smaller, faster
        # starting binary than thin/4. The extra link time is paid once here.
        doCheck = false;
        CARGO_PROFILE_RELEASE_LTO = "fat";
        CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1";

        nativeBuildInputs = [
          pkgs.pkg-config
          pkgs.makeWrapper
          pkgs.librsvg # rsvg-convert for the hicolor PNG sizes
          pkgs.desktop-file-utils # desktop-file-validate
        ];

        buildInputs = [
          pkgs.openssl
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

        # Wrap by hand: add the app font, the audio-only mpv runtime binary,
        # the certificate bundle and (for the GPUI child only) the
        # Wayland/Vulkan runtime libraries.
        #
        # The headless `musika` core spawns `mpvAudio` over Unix-socket JSON
        # IPC and never loads a GUI library itself; it only spawns the wrapped
        # `musika-ui` sibling. Keeping the GPU/Wayland path on that UI wrapper
        # alone avoids leaking a display stack into headless daemons.
        postInstall = ''
          wrapProgram $out/bin/musika \
            --prefix PATH : ${pkgs.lib.makeBinPath [ mpvAudio ]} \
            --prefix XDG_DATA_DIRS : ${fontDataDirs} \
            --set-default MUSIKA_MPV_BIN ${mpvAudio}/bin/mpv \
            --set-default SSL_CERT_FILE ${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt \
            --set-default MUSIKA_ASSET_DIR $out/share/musika

          wrapProgram $out/bin/musika-ui \
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
        mpvAudio = mpvAudio;
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
        buildInputs =
          [
            kachePkg
          ]
          ++ (with pkgs; [
            rustc
            cargo
            rust-analyzer
            sqlite
            sqlite-analyzer
            sqls
            rustfmt
            libglvnd
            mpvAudio
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
          ]);

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (
          with pkgs;
          [
            wayland
            libxkbcommon
            libx11
            libxcursor
            libxrandr
            libxi
            libxcb
            fontconfig
            freetype
            libglvnd
            vulkan-loader
          ]
        );

        shellHook = ''
          export XDG_DATA_DIRS="${fontDataDirs}''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
          export MUSIKA_MPV_BIN="${mpvAudio}/bin/mpv"
          export RUSTC_WRAPPER="${kachePkg}/bin/kache"
        '';
      };
    };
}
