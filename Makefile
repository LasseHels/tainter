SHELL = /usr/bin/env bash
IMAGE_NAME?=tainter
IMAGE_VERSION?=latest
IMAGE_TAG?="$(IMAGE_NAME):$(IMAGE_VERSION)"
# See https://stackoverflow.com/a/71766578.
IMAGE_PLATFORMS?=$(shell docker system info --format '{{.OSType}}/{{.Architecture}}')
END_TO_END_TEST_KUBERNETES_VERSION?=v1.29.7

.PHONY: run
run:
	cargo run -- --config-file "./settings/tainter.toml"

.PHONY: test
test:
	cargo test

.PHONY: setup
setup:
	IMAGE_TAG=tainter:end-to-end make image
	minikube start --nodes 3 --profile tainter-end-to-end --kubernetes-version=$(END_TO_END_TEST_KUBERNETES_VERSION)
  # See https://minikube.sigs.k8s.io/docs/handbook/pushing/#7-loading-directly-to-in-cluster-container-runtime.
	minikube image load tainter:end-to-end --profile tainter-end-to-end
	kubectl kustomize ./tests | kubectl apply -f -
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
	@echo "Building image with tag $(IMAGE_TAG) and platform(s) $(IMAGE_PLATFORMS)"
  # We need to add --load to the command so that the built images show up in "docker image ls".
  # See https://github.com/docker/buildx/issues/59#issuecomment-1168619521.
	docker buildx build --load --tag $(IMAGE_TAG) --platform $(IMAGE_PLATFORMS) .

.PHONY: run-image
run-image:
	docker run --volume "./settings:/settings" $(IMAGE_TAG) --config-file="/settings/tainter.toml"

.PHONY: kubeconform
kubeconform:
	kubeconform \
    -schema-location default \
    -schema-location 'kubeconform/schemas/{{ .ResourceKind }}.json' \
    -strict \
    -summary deploy
