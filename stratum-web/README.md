# Stratum Web

The Compose frontend is available at `http://localhost:5173` and connects to
the local Stratum API at `http://127.0.0.1:18080`.

```bash
podman compose up --build
```

For local frontend development, run:

```bash
pnpm install
pnpm dev
pnpm typecheck
pnpm test
pnpm build
```
