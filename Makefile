# RollbackX Makefile
BINARY        := rollbackx
MUSL_BIN      := target/x86_64-unknown-linux-musl/release/rollbackx
GTK_BIN       := target/release/rollbackx-gtk
PREFIX        ?= /usr/local
DESTDIR       ?=

# .deb paket ayarları
DEB_VERSION   := 1.0.1
DEB_ARCH      := amd64
DEB_PKGNAME   := rollbackx_$(DEB_VERSION)_$(DEB_ARCH)
DEB_STAGEDIR  := /tmp/$(DEB_PKGNAME)

.PHONY: all build release install uninstall deb clean test

all: build

## Debug build
build:
	cargo build --workspace

## Release build
release:
	cargo build --release --workspace

## Testleri çalıştır
test:
	cargo test --workspace

## Release build + sistem geneline kur
install: release
	install -Dm755 $(RELEASE_BIN) $(DESTDIR)$(PREFIX)/bin/$(BINARY)
	install -Dm755 target/release/rollbackx-gtk $(DESTDIR)$(PREFIX)/bin/rollbackx-gtk
	install -Dm644 packaging/apt-hook/80rollbackx \
		$(DESTDIR)/etc/apt/apt.conf.d/80rollbackx
	install -Dm755 packaging/grub.d/80_rollbackx \
		$(DESTDIR)/etc/grub.d/80_rollbackx
	install -Dm644 packaging/polkit/org.rollbackx.policy \
		$(DESTDIR)/usr/share/polkit-1/actions/org.rollbackx.policy
	install -Dm644 packaging/rollbackx.desktop \
		$(DESTDIR)/usr/share/applications/rollbackx.desktop
	install -Dm644 packaging/icons/rollbackx.svg \
		$(DESTDIR)/usr/share/icons/hicolor/scalable/apps/rollbackx.svg
	install -Dm644 packaging/rollbackx-gtk-autostart.desktop \
		$(DESTDIR)/etc/xdg/autostart/rollbackx-gtk.desktop
	install -Dm755 packaging/systemd/cmdline-restore.sh \
		$(DESTDIR)/usr/lib/rollbackx/cmdline-restore.sh
	install -Dm644 packaging/systemd/rollbackx-cmdline.service \
		$(DESTDIR)/lib/systemd/system/rollbackx-cmdline.service
	mkdir -p $(DESTDIR)/var/lib/rollbackx
	chmod 755 $(DESTDIR)/var/lib/rollbackx
	systemctl daemon-reload 2>/dev/null || true
	systemctl enable rollbackx-cmdline.service 2>/dev/null || true
	@echo "Kurulum tamamlandı. GRUB güncelleniyor..."
	-update-grub 2>/dev/null || true

## Sistem genelinden kaldır
uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/$(BINARY)
	rm -f $(DESTDIR)$(PREFIX)/bin/rollbackx-gtk
	rm -f $(DESTDIR)/etc/apt/apt.conf.d/80rollbackx
	rm -f $(DESTDIR)/etc/grub.d/80_rollbackx
	rm -f $(DESTDIR)/usr/share/polkit-1/actions/org.rollbackx.policy
	rm -f $(DESTDIR)/usr/share/applications/rollbackx.desktop
	rm -f $(DESTDIR)/etc/xdg/autostart/rollbackx-gtk.desktop
	rm -f $(DESTDIR)/usr/share/icons/hicolor/scalable/apps/rollbackx.svg
	rm -f $(DESTDIR)/usr/lib/rollbackx/cmdline-restore.sh
	rm -f $(DESTDIR)/lib/systemd/system/rollbackx-cmdline.service
	systemctl disable rollbackx-cmdline.service 2>/dev/null || true
	-update-grub 2>/dev/null || true

## .deb paketi oluştur (dpkg-deb gerekli — Debian/Pardus'ta varsayılan kurulu)
## GTK libs varsa (pkg-config gtk4) otomatik derlenir, yoksa atlanır.
deb:
	@echo "==> CLI musl build yapılıyor..."
	cargo build --release --target x86_64-unknown-linux-musl -p rollbackx-cli

	@echo "==> GTK build deneniyor..."
	@if pkg-config --exists gtk4 libadwaita-1 2>/dev/null; then \
	    echo "    GTK libs bulundu, derleniyor..."; \
	    cargo build --release -p rollbackx-gtk; \
	else \
	    echo "    [UYARI] GTK libs bulunamadı (libgtk-4-dev eksik), GTK atlandı."; \
	fi

	@echo "==> Paket dizini hazırlanıyor: $(DEB_STAGEDIR)"
	rm -rf $(DEB_STAGEDIR)
	mkdir -p $(DEB_STAGEDIR)/DEBIAN
	mkdir -p $(DEB_STAGEDIR)/usr/bin
	mkdir -p $(DEB_STAGEDIR)/usr/lib/rollbackx
	mkdir -p $(DEB_STAGEDIR)/usr/share/applications
	mkdir -p $(DEB_STAGEDIR)/usr/share/polkit-1/actions
	mkdir -p $(DEB_STAGEDIR)/etc/grub.d
	mkdir -p $(DEB_STAGEDIR)/etc/xdg/autostart
	mkdir -p $(DEB_STAGEDIR)/lib/systemd/system
	mkdir -p $(DEB_STAGEDIR)/var/lib/rollbackx
	mkdir -p $(DEB_STAGEDIR)/usr/lib/systemd/system

	# CLI binary (musl statik — glibc bağımsız)
	install -m755 $(MUSL_BIN) $(DEB_STAGEDIR)/usr/bin/rollbackx

	# GTK binary (dinamik — varsa dahil et)
	[ -f $(GTK_BIN) ] && install -m755 $(GTK_BIN) $(DEB_STAGEDIR)/usr/bin/rollbackx-gtk || \
	    echo "  [UYARI] GTK binary bulunamadı, atlandı: $(GTK_BIN)"

	# Restore scriptleri
	install -m755 packaging/systemd/do-restore.sh      $(DEB_STAGEDIR)/usr/lib/rollbackx/do-restore.sh
	install -m755 packaging/systemd/cmdline-restore.sh $(DEB_STAGEDIR)/usr/lib/rollbackx/cmdline-restore.sh

	# GRUB script
	install -m755 packaging/grub.d/80_rollbackx $(DEB_STAGEDIR)/etc/grub.d/80_rollbackx

	# Systemd servisleri
	install -m644 packaging/systemd/rollbackx-restore.service  $(DEB_STAGEDIR)/lib/systemd/system/rollbackx-restore.service
	install -m644 packaging/systemd/rollbackx-cmdline.service  $(DEB_STAGEDIR)/lib/systemd/system/rollbackx-cmdline.service
	install -m644 packaging/systemd/rollbackx-schedule.service $(DEB_STAGEDIR)/lib/systemd/system/rollbackx-schedule.service
	install -m644 packaging/systemd/rollbackx-schedule.timer   $(DEB_STAGEDIR)/lib/systemd/system/rollbackx-schedule.timer

	# Masaüstü entegrasyonu
	install -m644 packaging/rollbackx.desktop               $(DEB_STAGEDIR)/usr/share/applications/rollbackx.desktop
	install -m644 packaging/rollbackx-gtk-autostart.desktop $(DEB_STAGEDIR)/etc/xdg/autostart/rollbackx-gtk.desktop
	install -m644 packaging/polkit/org.rollbackx.policy     $(DEB_STAGEDIR)/usr/share/polkit-1/actions/org.rollbackx.policy

	# Uygulama ikonu
	mkdir -p $(DEB_STAGEDIR)/usr/share/icons/hicolor/scalable/apps
	install -m644 packaging/icons/rollbackx.svg $(DEB_STAGEDIR)/usr/share/icons/hicolor/scalable/apps/rollbackx.svg

	# DEBIAN metadata
	install -m644 packaging/debian/control  $(DEB_STAGEDIR)/DEBIAN/control
	install -m755 packaging/debian/postinst $(DEB_STAGEDIR)/DEBIAN/postinst
	install -m755 packaging/debian/prerm    $(DEB_STAGEDIR)/DEBIAN/prerm

	@echo "==> Paket derleniyor..."
	dpkg-deb --build $(DEB_STAGEDIR) $(DEB_PKGNAME).deb

	@echo ""
	@echo "==> Paket hazır: $(DEB_PKGNAME).deb"
	@echo "    Kurmak için  : sudo dpkg -i $(DEB_PKGNAME).deb"
	@echo "    Kaldırmak için: sudo dpkg -r rollbackx"

## Temizle
clean:
	cargo clean

## VM test ortamlarını kur (root gerekli)
vm-setup-btrfs:
	sudo bash tests/vm-setup/setup-btrfs.sh

vm-setup-lvm:
	sudo bash tests/vm-setup/setup-lvm.sh

## Lint
lint:
	cargo clippy --workspace -- -D warnings

## Format
fmt:
	cargo fmt --all
