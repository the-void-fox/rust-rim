##############################################################################
# RustRim — Makefile
# Использование (внутри nix-shell):
#   make linux      — нативный Linux бинарник
#   make appimage   — AppImage для Linux
#   make windows    — .exe для Windows
#   make all        — все три
#   make clean      — удалить dist/ и tools/
##############################################################################

# Имя бинарника (cargo: дефисы → подчёркивания)
CARGO_BIN  := rust-rim
# Имя артефактов в dist/
APP_NAME   := rust-rim
APP_VER    := $(shell grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
DIST       := dist

TARGET_LIN := x86_64-unknown-linux-gnu
TARGET_WIN := x86_64-pc-windows-gnu

BIN_LIN    := target/$(TARGET_LIN)/release/$(CARGO_BIN)
BIN_WIN    := target/$(TARGET_WIN)/release/$(CARGO_BIN).exe

# appimagetool скачиваем как AppImage в tools/ (его нет в nixpkgs)
APPIMAGETOOL_URL := https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage
APPIMAGETOOL     := tools/appimagetool.AppImage

.PHONY: all linux appimage windows clean sizes _appdir _bundle_libs

all: linux appimage windows

# ── 1. Linux native ───────────────────────────────────────────────────────────
linux: $(DIST)/$(APP_NAME)-$(APP_VER)-linux-x86_64

$(DIST)/$(APP_NAME)-$(APP_VER)-linux-x86_64: $(BIN_LIN)
	@mkdir -p $(DIST)
	cp $(BIN_LIN) $@
	@echo "✅  Linux → $@"

# FORCE: инкрементальностью управляет cargo, а не make —
# иначе устаревший бинарник в target/ попадает в dist/ без пересборки.
$(BIN_LIN): FORCE
	cargo zigbuild --release --target $(TARGET_LIN)
	# Убираем /nix/store пути из RPATH — бинарник должен работать
	# на обычных Linux системах (Debian, Ubuntu, Arch, …)
	patchelf --set-rpath '$$ORIGIN/lib:/usr/lib:/usr/lib/x86_64-linux-gnu' $@


# ── 2. AppImage ───────────────────────────────────────────────────────────────
appimage: $(DIST)/$(APP_NAME)-$(APP_VER)-x86_64.AppImage

# Скачиваем appimagetool если его ещё нет
$(APPIMAGETOOL):
	@echo "→ Скачиваю appimagetool…"
	@mkdir -p tools
	wget -q --show-progress -O $(APPIMAGETOOL) $(APPIMAGETOOL_URL)
	chmod +x $(APPIMAGETOOL)

$(DIST)/$(APP_NAME)-$(APP_VER)-x86_64.AppImage: $(BIN_LIN) $(APPIMAGETOOL)
	@mkdir -p $(DIST)
	$(MAKE) _appdir
	# appimage-run позволяет запустить .AppImage на NixOS
	# SOURCE_DATE_EPOCH конфликтует с appimagetool на NixOS — сбрасываем
	env -u SOURCE_DATE_EPOCH ARCH=x86_64 appimage-run $(APPIMAGETOOL) AppDir \
	    $(DIST)/$(APP_NAME)-$(APP_VER)-x86_64.AppImage
	rm -rf AppDir
	@echo "✅  AppImage → $(DIST)/$(APP_NAME)-$(APP_VER)-x86_64.AppImage"

_appdir: $(BIN_LIN)
	rm -rf AppDir
	mkdir -p AppDir/usr/bin AppDir/usr/lib

	# Бинарник
	cp $(BIN_LIN) AppDir/usr/bin/$(APP_NAME)
	patchelf --set-rpath '$$ORIGIN/../lib' AppDir/usr/bin/$(APP_NAME)

	# Копируем .so зависимости (кроме базовых системных)
	$(MAKE) _bundle_libs

	# Иконка
	mkdir -p AppDir/usr/share/icons/hicolor/256x256/apps
	cp src/assets/icon.png \
	    AppDir/usr/share/icons/hicolor/256x256/apps/$(APP_NAME).png
	ln -sf usr/share/icons/hicolor/256x256/apps/$(APP_NAME).png \
	    AppDir/$(APP_NAME).png

	# .desktop
	printf '[Desktop Entry]\nType=Application\nName=RustRim\nExec=rust-rim\nIcon=rust-rim\nCategories=Game\nComment=RimWorld mod manager\n' \
	    > AppDir/$(APP_NAME).desktop

	# AppRun — враппер, прописывает LD_LIBRARY_PATH внутри AppImage
	printf '#!/bin/sh\nHERE="$$(dirname "$$(readlink -f "$$0")")"\nexport LD_LIBRARY_PATH="$$HERE/usr/lib:$$LD_LIBRARY_PATH"\nexec "$$HERE/usr/bin/$(APP_NAME)" "$$@"\n' \
	    > AppDir/AppRun
	chmod +x AppDir/AppRun

# Собираем зависимости через ldd, исключаем базовые (libc, libm и т.д.)
_bundle_libs:
	@echo "→ Bundling .so files…"
	@ldd AppDir/usr/bin/$(APP_NAME) \
	  | grep "=> /" \
	  | awk '{print $$3}' \
	  | grep -Ev '/(libc|libm|libdl|libpthread|librt|libresolv|ld-linux)[.-]' \
	  | while read lib; do \
	      echo "  + $$lib"; \
	      cp -L "$$lib" AppDir/usr/lib/ 2>/dev/null || true; \
	    done


# ── 3. Windows (.exe) ─────────────────────────────────────────────────────────
windows: $(DIST)/$(APP_NAME)-$(APP_VER)-windows-x86_64.exe

$(DIST)/$(APP_NAME)-$(APP_VER)-windows-x86_64.exe: $(BIN_WIN)
	@mkdir -p $(DIST)
	cp $(BIN_WIN) $@
	@echo "✅  Windows → $@"

$(BIN_WIN): FORCE
	# cargo-zigbuild использует Zig как линкер — не требует Docker
	# -mwindows скрывает консоль (аналог windows_subsystem = "windows")
	cargo zigbuild --release --target $(TARGET_WIN)


# ── Утилиты ───────────────────────────────────────────────────────────────────
clean:
	rm -rf $(DIST) AppDir tools
	@echo "🧹  Готово"

sizes:
	@echo "\n── Артефакты ──"
	@ls -lh $(DIST)/ 2>/dev/null || echo "(пусто)"
FORCE:
