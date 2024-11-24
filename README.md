# Tainter
Tainter automatically taints Kubernetes nodes based on node conditions.

## Motivation

Tainter was originally conceived to add the well-known [node.kubernetes.io/out-of-service:NoExecute](https://kubernetes.io/docs/reference/labels-annotations-taints/#node-kubernetes-io-out-of-service) taint to preempted spot nodes.

We found that kubelet on preempted nodes did not always have time to detach volumes before the node was reclaimed.
When this happened, pods with attached [ReadWriteOnce](https://kubernetes.io/docs/concepts/storage/persistent-volumes/#access-modes)
volumes were unable to re-schedule on a different node because their volume was still attached to the reclaimed node.
Pods were stuck in this state for [six minutes](https://github.com/kubernetes/kubernetes/blob/d9c54f69d4bb7ae1bb655e1a2a50297d615025b5/pkg/controller/volume/attachdetach/attach_detach_controller.go#L96)
until Kubernetes' attach detach controller [forcefully detached the volume](https://github.com/kubernetes/kubernetes/blob/d9c54f69d4bb7ae1bb655e1a2a50297d615025b5/pkg/controller/volume/attachdetach/reconciler/reconciler.go#L279).

By adding the [node.kubernetes.io/out-of-service:NoExecute](https://kubernetes.io/docs/reference/labels-annotations-taints/#node-kubernetes-io-out-of-service)
taint to preempted nodes, the attach detach controller [forcefully detach volumes](https://github.com/kubernetes/kubernetes/blob/d9c54f69d4bb7ae1bb655e1a2a50297d615025b5/pkg/controller/volume/attachdetach/reconciler/reconciler.go#L285)
from the node, and pods can be immediately re-scheduled without having to wait for the [six-minute timeout](https://github.com/kubernetes/kubernetes/blob/d9c54f69d4bb7ae1bb655e1a2a50297d615025b5/pkg/controller/volume/attachdetach/attach_detach_controller.go#L96).

While originally created for this use-case, Tainter can be configured to add any taint when a node matches any conditions.

## Configuration

Tainter expects a `--config-file` argument with the path to Tainter's TOML configuration file.

Example configuration:
```toml
# HTTP server that exposes Tainter's /health endpoint.
[server]
host = "0.0.0.0"
port = "8080"

[log]
# The maximum level at which to output logs.
max_level = "info"

[[reconciler.matchers]]
# Add this taint to any node that has both of the below conditions.
[reconciler.matchers.taint]
# Tainter will automatically add a time_added field to the taint if effect is "NoExecute".
# See https://kubernetes.io/docs/reference/generated/kubernetes-api/v1.27/#taint-v1-core.
effect = "NoExecute"
key = "pressure"
value = "memory"

[[reconciler.matchers.conditions]]
type = "NetworkInterfaceCard"
# Status is a regular expression.
status = "Kaput|Ruined"

[[reconciler.matchers.conditions]]
type = "PrivateLink"
status = "severed"
```

## Run

Run Tainter locally with `make run`.

## Release

An image is automatically built on all pushes to `main` as well as when a new tag is pushed. To release a new version
of Tainter, create an annotated tag and push it:
```shell
git tag -a <VERSION> -m "<DESCRIPTION>"
git push origin --tags
```
Release versions follow [semantic versioning](https://semver.org). Do not prefix versions with `v`; i.e., `1.0.0`
instead of `v1.0.0`

Tainter images are stored in the https://hub.docker.com/r/lassehels/tainter repository.

## Deploy

Tainter is designed to be deployed in a Kubernetes cluster. Tainter needs `list`, `watch` and `update` permissions on
the `nodes` resource. Example Tainter manifest files are found in the [deploy](deploy) directory.
Run `make manifest` to generate a single `tainter.yaml` file with all the necessary Kubernetes resources needed to run
Tainter.
