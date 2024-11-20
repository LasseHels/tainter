IMAGE_NAME?=tainter
IMAGE_VERSION?=latest
IMAGE_TAG?="$(IMAGE_NAME):$(IMAGE_VERSION)"

.PHONY: run
run:
	cargo run -- --config-file "./settings/tainter.toml"

.PHONY: test
test:
	cargo test

.PHONY: setup
setup:
	minikube start --nodes 3 --profile tainter-end-to-end --kubernetes-version=v1.29.7
	kubectl kustomize ./deploy | kubectl apply -f -
	kubectl proxy --port=8011 &

.PHONY: teardown
teardown:
	minikube delete --profile tainter-end-to-end
	killall kubectl proxy

.PHONY: test-end-to-end
test-end-to-end:
	cargo test -- --show-output --ignored

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
