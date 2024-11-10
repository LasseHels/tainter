IMAGE_NAME?=tainter
IMAGE_VERSION?=latest
IMAGE_TAG?="$(IMAGE_NAME):$(IMAGE_VERSION)"

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

.PHONY: image
image:
	@echo "Building image with tag $(IMAGE_TAG)"
	docker build -t $(IMAGE_TAG) .

.PHONY: run-image
run-image:
	docker run --volume "./settings:/settings" $(IMAGE_TAG) --config-file="/settings/tainter.toml"
