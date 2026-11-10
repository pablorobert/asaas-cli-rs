VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
PREFIX  ?= $(HOME)/.local

# Read by build.rs and embedded in `asaas version`.
export ASAAS_VERSION := $(VERSION)

.PHONY: build install uninstall test lint

build:
	cargo build --release --locked
	mkdir -p bin
	cp target/release/asaas bin/asaas

install: build
	mkdir -p $(PREFIX)/bin
	install -m 0755 target/release/asaas $(PREFIX)/bin/asaas

uninstall:
	rm -f $(PREFIX)/bin/asaas

test:
	cargo test --locked

lint:
	cargo fmt --check
	cargo clippy --locked --all-targets -- -D warnings
