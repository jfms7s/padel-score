# padel-score

A pure Rust padel scoring engine (`padel-scoring`): rules, match state and replay from an event log.
No clock, threads, randomness or I/O, so it also builds for `wasm32-unknown-unknown`.

## Develop

```sh
cargo test
cargo build -p padel-scoring --target wasm32-unknown-unknown
```

## Run locally

The image holds the server with the web app embedded in it. The SQLite database lives in `/data`.

```sh
docker build --build-arg APP_VERSION=v0.0.0-dev -t padel-score:dev .
docker run --rm -p 8080:8080 -v padel-score-dev:/data padel-score:dev
```

Then open <http://localhost:8080>. With `podman build`, add `--format docker`, or the `HEALTHCHECK` is dropped.

## Accounts

Anonymous quick matches need no account. Owned matches, invites, the admin pages and watch sign-in do.

**First run.** While there are no users, the server logs a one-time setup code (a new one on each restart until the first admin exists). Find it, then open `/setup`:

```sh
kubectl logs -n padel-score deploy/padel-score-padel-score-server | grep "setup code"
```

Or set `SETUP_TOKEN` and use that value as the setup code. The setup creates the first admin.

**Invites.** There is no public sign-up. An admin creates an invite on the admin page and sends the link (`/invite#<code>`) to the person. The code is in the URL fragment, so it never reaches a server log. Invites expire and can be revoked. An admin can also disable and re-enable users.

**Password resets.** An admin creates a reset link (`/reset#<code>`) for a user and sends it to them. Using it sets a new password and signs the user out everywhere.

**Known gap: no email.** The server sends no email, so a forgotten password is recovered through an admin-made reset link, never by the user alone. If the only admin loses their password, a reset needs database access.

**Watch sign-in.** The watch shows a code; sign in on the web and enter it at `/link` to approve the device. A signed-in watch is listed on the account page and can be revoked there.

Expired sessions, invites, device codes and reset codes are deleted hourly, together with anonymous matches idle for 30 days. Owned matches are never purged.

Environment variables:

| Variable | Default | Meaning |
| --- | --- | --- |
| `SECURE_COOKIES` | on | The session cookie is `__Host-padel_session` with `Secure`. Set `false` or `0` for plain-HTTP development, e.g. `docker run -e SECURE_COOKIES=false`. |
| `TRUSTED_PROXIES` | empty | Comma-separated CIDRs or IPs of reverse proxies whose `X-Forwarded-For` is believed. Empty: the TCP peer is the client, which behind a proxy puts every user in one rate-limit bucket. A bad entry stops startup. |
| `PUBLIC_ORIGIN` | unset | The public URL, e.g. `https://padel.example.com`, no trailing slash. The CSRF check compares a request's `Origin` with it. Unset: it compares the `Origin` host with `Host`. |
| `SETUP_TOKEN` | unset | The setup code for the first admin. Unset: a random one-time code is logged. |

## Releases

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`. If the tagged commit is on `main` and every CI check passes on it, the workflow builds `ghcr.io/jfms7s/padel-score:vX.Y.Z`
for `linux/amd64` and `linux/arm64` (cross-compiled, no emulation) and pushes it with signed build provenance:

```sh
gh attestation verify oci://ghcr.io/jfms7s/padel-score:vX.Y.Z --repo jfms7s/padel-score
```
