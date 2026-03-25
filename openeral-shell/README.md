# openeral-shell (Legacy)

`openeral-shell/` is the older entrypoint-driven sandbox path.

It is not the supported OpenShell deployment model anymore.

## Supported Path

Use these instead:

- [`../sandboxes/openeral/README.md`](../sandboxes/openeral/README.md) for the published sandbox image
- [`../README.md`](../README.md) for the top-level product and OpenShell flow
- [`../vendor/openshell`](../vendor/openshell) for the custom cluster image source

The current OpenShell architecture is:

- custom cluster image deploys the FUSE device plugin and configures the gateway to request `github.com/fuse`
- published sandbox image declares `fuse.openeral` mounts in `/etc/fstab`
- side-loaded `openshell-sandbox` supervisor mounts `/db` and `/home/agent`
- `/home/agent` is keyed by `OPENSHELL_SANDBOX_ID`

## Why This Directory Still Exists

This directory is kept only as a legacy local image path and reference for older experiments.

Do not use it for:

- product documentation
- verification of the current OpenShell flow
- published deployment instructions

Do not treat `openeral-shell-start.sh`, `.env` upload, or container `ENTRYPOINT` startup as the current OpenShell model. The supported flow is supervisor-managed FUSE from `/etc/fstab`.
