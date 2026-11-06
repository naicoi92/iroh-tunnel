# Docker and Kubernetes

The release images are distroless, multi-arch (`linux/amd64`, `linux/arm64`)
and come in three flavors, one per executable; see
[Container images](install.md#container-images). Their entrypoint is the
executable, so arguments start at the role or command.

- [Running the image](#running-the-image)
- [Compose demo](#compose-demo)
- [Kubernetes](#kubernetes)
- [Notes for containers](#notes-for-containers)

## Running the image

Quick mode needs no file:

```sh
docker run --rm ghcr.io/naicoi92/iroh-tunnel-serve:v0.5.0 \
  -n web -t 192.0.2.10:80
```

With a config file, mount it read-only. It must already contain a
`secret_key` (`config keygen --print` prints one), because `run` cannot save a
generated key into a read-only file:

```sh
docker run --rm \
  -v "$PWD/serve.toml:/etc/iroh-tunnel/serve.toml:ro" \
  ghcr.io/naicoi92/iroh-tunnel:v0.5.0 \
  serve run --config /etc/iroh-tunnel/serve.toml
```

## Compose demo

[`docker-compose.yml`](../docker-compose.yml) builds the image from source and
tunnels an nginx container through Serve and Access. The sample configs in
[`examples/`](../examples/) ship a throwaway demo key, and `access.toml`
already points at its NodeId.

```sh
docker compose up -d web serve
docker compose logs serve | grep NodeId
# NodeId: e8bb34671b11ca03ca88f7f8b500b07ab3086be8cfd168b3d041d3f478837de9
docker compose up -d access
curl http://127.0.0.1:8080        # nginx default page, through the tunnel
docker compose --profile demo down
```

`access` is in the `demo` profile, so it starts only when named. Never reuse
the demo key outside this demo.

## Kubernetes

Run Serve as a Deployment next to an in-cluster Service. Iroh dials out to
its relays, so the pod needs only normal egress: no `Service`, `Ingress` or
host networking in front of it.

Generate a key once and keep the whole config in a Secret, so the NodeId
survives restarts and rollouts:

```sh
iroh-tunnel serve config keygen --print
# Secret key: <base64>
# Node ID: <hex>     <- the NodeId clients dial
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: iroh-tunnel-serve
stringData:
  serve.toml: |
    [node]
    secret_key = "<base64 from keygen --print>"

    [[services]]
    name = "postgres"
    protocol = "tcp"
    host = "postgres.default.svc.cluster.local"
    port = 5432
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: iroh-tunnel-serve
spec:
  replicas: 1
  selector:
    matchLabels:
      app: iroh-tunnel-serve
  template:
    metadata:
      labels:
        app: iroh-tunnel-serve
    spec:
      containers:
        - name: serve
          image: ghcr.io/naicoi92/iroh-tunnel-serve:v0.5.0
          args: ["run", "--config", "/etc/iroh-tunnel/serve.toml"]
          volumeMounts:
            - name: config
              mountPath: /etc/iroh-tunnel
              readOnly: true
      volumes:
        - name: config
          secret:
            secretName: iroh-tunnel-serve
```

```sh
kubectl apply -f iroh-tunnel-serve.yaml
kubectl logs deploy/iroh-tunnel-serve | grep NodeId
```

On your workstation, dial it:

```sh
iroh-tunnel access -n postgres -l 127.0.0.1:15432 -N <NODE_ID>
psql -h 127.0.0.1 -p 15432
```

Keep `replicas: 1`: two pods with the same key would claim the same NodeId.

## Notes for containers

- **Updates.** Images set `IROH_TUNNEL_CONTAINER=1`, so `update` never
  replaces the binary; pull a new tag and redeploy.
- **Health.** There is no HTTP health endpoint. The role is the container's
  main process, so the container exits when the role stops and the restart
  policy takes over. `status --json` inside the container shows the live
  paths.
- **State.** The status file and role lock live under
  `$HOME/.local/state/iroh-tunnel`; the release images set `HOME=/tmp`.
