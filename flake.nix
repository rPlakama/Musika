{
  description = "Musika — keyboard-driven Subsonic/Navidrome player (Rust + Qt6/QML)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      qt = pkgs.qt6.env "qt6-env" (
        with pkgs.qt6;
        [
          qtbase
          qtdeclarative
          qtwayland
        ]
      );

      # App-scoped typeface: exposed to fontconfig through XDG_DATA_DIRS (by
      # the wrapper below and the dev shell), never installed system-wide.
      # Overused Grotesk stands in for the unpackaged Bricolage Grotesque and
      # is the single face for wordmark, lyrics and body (Theme.fonts).
      fonts = with pkgs; [
        overused-grotesk
      ];
      fontDataDirs = pkgs.lib.makeSearchPath "share" fonts;

      # pkgs.qt6.env merges every Qt module into one prefix and drops a qt.conf
      # next to qmake, so qmake reports the merged prefix. qtpaths, however, is
      # left as a symlink to qtbase's binary; Qt resolves that symlink, finds no
      # qt.conf there and falls back to the qtbase prefix. qtbridge resolves each
      # module as "$QT_INSTALL_HEADERS/Qt<Module>", so QtQml/QtQuickTest end up
      # under qtbase and the build fails.
      #
      # Pin qtpaths to the merged qt.conf, and make qmake advertise that wrapper
      # as QT_HOST_BINS -- which is where qtbridge looks for the qtpaths tool.
      qtpathsFixed = pkgs.writeShellScriptBin "qtpaths" ''
        exec ${qt}/bin/qtpaths6 --qtconf ${qt}/bin/qt.conf "$@"
      '';

      qmakeFixed = pkgs.writeShellScriptBin "qmake" ''
        if [ "$1" = "-query" ]; then
          case "$2" in
            QT_HOST_BINS|QT_HOST_BINS/get|QT_INSTALL_BINS|QT_INSTALL_BINS/get)
              echo "${qtpathsFixed}/bin"
              exit 0
              ;;
          esac
        fi
        exec ${qt}/bin/qmake "$@"
      '';

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
              ".qmlls.ini"
            ]);
        };

        cargoLock.lockFile = ./Cargo.lock;

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
          qt
        ];

        QMAKE = "${qmakeFixed}/bin/qmake";
        QT_VERSION_MAJOR = "6";

        # The sandbox has no /etc/ssl; reqwest builds a client even in tests.
        SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

        # Wrap by hand: the merged `qt` env is what qtbridge resolves against,
        # and adding individual Qt modules breaks that discovery. The wrapper
        # mirrors the dev-shell environment (plugins, QML imports) and puts the
        # runtime playback engine (mpv) on PATH.
        postInstall = ''
          wrapProgram $out/bin/musika \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.mpv ]} \
            --prefix XDG_DATA_DIRS : ${fontDataDirs} \
            --prefix QT_PLUGIN_PATH : ${qt}/lib/qt-6/plugins \
            --prefix QML_IMPORT_PATH : ${qt}/lib/qt-6/qml \
            --prefix QML2_IMPORT_PATH : ${qt}/lib/qt-6/qml \
            --set-default SSL_CERT_FILE ${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt

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
          rustPlatform.bindgenHook # drop if no crate needs libclang -- which is prob the case;;
        ];

        # Should be a proper way to organize this, perhaps when in build time to actually adjust
        # the shell, since it exposes all, in case which can, or cannot, be used at all.
        buildInputs = with pkgs; [
          rustc
          cargo
          ffmpeg_4
          mpv
          rust-analyzer
          sqlite
          sqlite-analyzer
          sqls
          rustfmt
          libglvnd
          openssl
          sqlx-cli
          qt
        ];

        QMAKE = "${qmakeFixed}/bin/qmake";
        QT_VERSION_MAJOR = "6";

        shellHook = ''
          	export QML_IMPORT_PATH="${qt}/lib/qt-6/qml''${QML_IMPORT_PATH:+:$QML_IMPORT_PATH}"
          	export QML2_IMPORT_PATH="${qt}/lib/qt-6/qml''${QML2_IMPORT_PATH:+:$QML2_IMPORT_PATH}"
          	export QT_PLUGIN_PATH="${qt}/lib/qt-6/plugins''${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}"
          	export XDG_DATA_DIRS="${fontDataDirs}''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
          		cat > .qmlls.ini <<EOF
          [General]
          no-cmake-calls=true
          importPaths=${qt}/lib/qt-6/qml
          EOF
        '';
      };
    };
}
