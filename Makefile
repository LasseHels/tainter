.PHONY: run
run:
	cargo run -- --config-file "./settings/tainter.toml"

.PHONY: test
test:
	cargo test

.PHONY: fmt
fmt:
	cargo fmt

.PHONY: lint
lint:
	cargo clippy

.PHONY: build
build:
	cargo build
