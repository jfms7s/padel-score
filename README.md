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

**Invites.** There is no public sign-up. Any signed-in user creates an invite on `/account` and sends the link (`/invite#<code>`) to the person. The code is in the URL fragment and then in request bodies, never in a URL path, so it never reaches a server log. Invites expire and can be revoked.

**Admin.** The admin page lists the users, disables and re-enables them, and makes password reset links.

**Password resets.** An admin creates a reset link (`/reset#<code>`) for a user and sends it to them. Using it sets a new password and signs the user out everywhere.

**Known gap: no email.** The server sends no email, so a forgotten password is recovered through an admin-made reset link, never by the user alone. If the only admin loses their password, a reset needs database access.

**Known gap: per-address limits are global behind the homelab's load balancer.** Behind the homelab's Traefik ServiceLB, every visitor reaches the app with the same in-cluster address, so the per-address rate limits (login, device start and polling, invite and reset codes) are in effect one shared bucket. The per-email login limit still protects each account. The proper fix (`externalTrafficPolicy: Local`) comes before the app is public.

**Watch sign-in.** The watch shows a code; sign in on the web and enter it at `/link` to approve the device. A signed-in watch is listed on the account page and can be revoked there.

Expired sessions, invites, device codes and reset codes are deleted hourly, together with anonymous matches idle for 30 days. Owned matches are never purged.

## Players and history

Signed in, you also get:

- **Players** (`/players`): a list of the people you play with. Add, rename, archive and restore them; archived players keep their record. Nothing is ever deleted.
- **Picking players for a match**: on New match, each name field suggests players from your list, or adds the typed name to it. A typed name that isn't picked stays free text and is never added on its own.
- **History** (`/history`): your finished matches, newest first, with the winner, the score and a link to the final view.
- **Player pages** (`/players/{id}`): a player's wins, losses and draws, any placings, and their matches, each marked won, lost or drawn.

Records come from the match's own score, so an undo that un-finishes a match takes it out of history and the records again. Anonymous matches are in no one's history.

Environment variables:

| Variable | Default | Meaning |
| --- | --- | --- |
| `SECURE_COOKIES` | on | The session cookie is `__Host-padel_session` with `Secure`. Set `false` or `0` for plain-HTTP development, e.g. `docker run -e SECURE_COOKIES=false`. |
| `TRUSTED_PROXIES` | empty | Comma-separated CIDRs or IPs of reverse proxies whose `X-Forwarded-For` is believed. Empty: the TCP peer is the client, which behind a proxy puts every user in one rate-limit bucket. A bad entry stops startup. |
| `PUBLIC_ORIGIN` | unset | The public URL, e.g. `https://padel.example.com`, no trailing slash. The CSRF check compares a request's `Origin` with it. Unset: it compares the `Origin` host with `Host`. |
| `SETUP_TOKEN` | unset | The setup code for the first admin. Unset: a random one-time code is logged. |

## Events

Signed in, you can also run an event (`/events`): a tournament or a non-stop, with its own teams, courts and results.

- **Teams and courts**: pick two of your players per team (a player is in one team per event), and add courts. Each court has two QR codes: one to score, one to watch.
- **Court phones** (`/c/{token}`): one phone at each court's fence scores every match on that court. Tap Join, choose who serves, then score as usual; offline taps queue as on any scoring link. When a match ends, its score stays on screen with a "Next match" button. A final score can also be typed (`6-4`, `7-6(5)`).
- **Coordinator**: create matches, put them on courts or queue them, type any match's result (which then wins over the court phone), give a court a new scoring code, and finish the event by putting the teams in order (ties allowed). Finishing records each player's placing.
- **Watching** (`/cv/{token}` for one court, `/e/{token}` for the whole event): live scores, results and final positions, with no account and no way to change a score.

Event matches show in history and on player pages as "Sunday cup · Match 1".

## Releases

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`. If the tagged commit is on `main` and every CI check passes on it, the workflow builds `ghcr.io/jfms7s/padel-score:vX.Y.Z`
for `linux/amd64` and `linux/arm64` (cross-compiled, no emulation) and pushes it with signed build provenance:

```sh
gh attestation verify oci://ghcr.io/jfms7s/padel-score:vX.Y.Z --repo jfms7s/padel-score
```
