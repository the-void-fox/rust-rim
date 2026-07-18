{
  description = "RustRim — быстрый менеджер модов RimWorld (egui)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems
        (system: f (import nixpkgs { inherit system; }));

      # Библиотеки, которые egui/eframe и rfd загружают в рантайме (dlopen)
      runtimeLibs = pkgs: with pkgs; [
        libGL
        libGLU
        libX11
        libXi
        libXcursor
        libXrandr
        wayland
        libxkbcommon
        dbus.lib      # libdbus-1.so.3 — файловые диалоги rfd (портал XDG)
      ];
    in
    {
      # Дев-шелл — тот же, что и nix-shell (rustup, кросс-компиляция, appimage)
      devShells = forAllSystems (pkgs: {
        default = import ./shell.nix { inherit pkgs; };
      });

      # nix build / nix run — сборка пакета тулчейном из nixpkgs
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "rust-rim";
          version = "1.2.0";

          src = nixpkgs.lib.cleanSource self;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
          buildInputs = runtimeLibs pkgs ++ [ pkgs.gtk3 pkgs.glib ];

          # GUI-приложению нужны рантайм-библиотеки в LD_LIBRARY_PATH
          postFixup = ''
            wrapProgram $out/bin/rust-rim \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}
          '';

          meta = with nixpkgs.lib; {
            description = "Быстрый менеджер модов RimWorld";
            homepage = "https://github.com/the-void-fox/rust-rim";
            license = licenses.mit;
            mainProgram = "rust-rim";
            platforms = systems;
          };
        };
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.system}.default}/bin/rust-rim";
        };
      });
    };
}
