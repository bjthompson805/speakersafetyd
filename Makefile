# SPDX-License-Identifier: GPL-2.0-only
# Copyright The Asahi Linux Contributors

BINDIR ?= /usr/bin
UNITDIR ?= /lib/systemd/system
UDEVDIR ?= /lib/udev/rules.d
SHAREDIR ?= /usr/share/

all:
	cargo build --release

install: install-data
	install -dDm0755 $(DESTDIR)/$(BINDIR)
	install -pm0755 target/release/speakersafetyd $(DESTDIR)/$(BINDIR)/speakersafetyd

install-data:
	install -dDm0755 $(DESTDIR)/$(UNITDIR)
	install -pm0644 speakersafetyd.service $(DESTDIR)/$(UNITDIR)/speakersafetyd.service
	install -dDm0755 $(DESTDIR)/$(UDEVDIR)
	install -pm0644 95-speakersafetyd.rules $(DESTDIR)/$(UDEVDIR)/95-speakersafetyd.rules
	install -dDm0755 $(DESTDIR)/$(SHAREDIR)/speakersafetyd/apple
	install -pm0644 -t $(DESTDIR)/$(SHAREDIR)/speakersafetyd/apple $(wildcard conf/apple/*)

# FreeBSD: make install-freebsd (works with make(1) and gmake)
FBSD_PREFIX ?= /usr/local
FBSD_SBINDIR ?= $(FBSD_PREFIX)/sbin
FBSD_CONFDIR ?= $(FBSD_PREFIX)/share/speakersafetyd/conf
FBSD_RCDIR ?= $(FBSD_PREFIX)/etc/rc.d

install-freebsd:
	install -d -m 0755 $(DESTDIR)$(FBSD_SBINDIR) $(DESTDIR)$(FBSD_CONFDIR)/apple $(DESTDIR)$(FBSD_RCDIR)
	install -m 0755 target/release/speakersafetyd $(DESTDIR)$(FBSD_SBINDIR)/speakersafetyd
	install -m 0644 conf/apple/*.conf $(DESTDIR)$(FBSD_CONFDIR)/apple/
	install -m 0555 rc.d/speakersafetyd $(DESTDIR)$(FBSD_RCDIR)/speakersafetyd

uninstall-freebsd:
	rm -f $(DESTDIR)$(FBSD_SBINDIR)/speakersafetyd $(DESTDIR)$(FBSD_RCDIR)/speakersafetyd
	rm -rf $(DESTDIR)$(FBSD_PREFIX)/share/speakersafetyd

uninstall:
	rm -f $(DESTDIR)/$(BINDIR)/speakersafetyd $(DESTDIR)/$(UNITDIR)/speakersafetyd.service $(DESTDIR)/$(UDEVDIR)/95-speakersafetyd.rules
	rm -rf $(DESTDIR)/$(SHAREDIR)/speakersafetyd

.PHONY: all install install-data uninstall install-freebsd uninstall-freebsd
