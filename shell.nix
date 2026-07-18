{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    # Rust управляется через rustup — НЕ через nixpkgs-пакеты rustc/cargo.
    # Это нужно для кросс-компиляции (rustup target add …)
    rustup

    # egui/eframe — X11
    libX11
    libXcursor
    libXrandr
    libXi
    libxcb

    # Wayland (опционально)
    wayland
    wayland-protocols
    libxkbcommon

    # OpenGL
    libGL
    libGLU

    # rfd (файловые диалоги)
    # rfd ≥0.16 загружает libdbus-1.so.3 через dlopen (портал XDG);
    # zenity — его fallback, если портал недоступен.
    dbus
    zenity
    gtk3
    glib
    pkg-config

    # ── Инструменты дистрибуции ───────────────────────────────────────────
    appimage-run   # запускает appimagetool.AppImage на NixOS
    patchelf       # патчим RPATH у собранного бинарника
    wget           # скачать appimagetool при первом make appimage

    # Кросс-компиляция под Windows без Docker
    zig
    cargo-zigbuild
  ];

  shellHook = ''
    export WINIT_UNIX_BACKEND=x11
    export TMPDIR=/tmp

    export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [
      pkgs.libGL
      pkgs.libGLU
      pkgs.libX11
      pkgs.libXi
      pkgs.libXcursor
      pkgs.libXrandr
      pkgs.wayland
      pkgs.libxkbcommon
      pkgs.dbus.lib   # libdbus-1.so.3 для файловых диалогов rfd
    ]}

    # rustup хранит тулчейны в ~/.rustup — убеждаемся что PATH правильный
    export PATH="$HOME/.cargo/bin:$PATH"

    # Устанавливаем stable если ещё не установлен
    if ! rustup toolchain list | grep -q stable; then
      echo "→ Устанавливаю Rust stable (первый запуск)…"
      rustup default stable
    fi

    # Добавляем таргеты для кросс-компиляции
    rustup target add x86_64-unknown-linux-gnu 2>/dev/null || true
    rustup target add x86_64-pc-windows-gnu    2>/dev/null || true

    echo ""
    echo "  ✅  Void Canvas build shell  ($(rustc --version))"
    echo "  make linux      — нативный Linux бинарник"
    echo "  make appimage   — AppImage (appimagetool скачается автоматически)"
    echo "  make windows    — Windows .exe (cargo-zigbuild + zig)"
    echo "  make all        — всё сразу"
    echo ""
  '';
}