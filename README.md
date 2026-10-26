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
- **Linking** (`/players`, `/link-player`): link a player to an account, either by a QR code the coordinator shows (the player confirms "That's me"), or by a claim the player sends from a match they played in ("That's me" on the match view) and the coordinator approves. Unlink any time; the coordinator's own records never change.
- **My results** (`/profile`): one record, placings and match list across every coordinator you've played for, your linked entries and claims, and who can see your history: only you, coordinators you've played for (the default), or anyone with your profile link (`/p/{token}`), which shows no other people's names and can be replaced at any time.

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
- **Tournament mode**: a knockout for 4, 8, 16 or 32 teams, drawn at random when the event starts (redraw until the first point). Optionally "losers keep playing", so every place is played for; otherwise the semi-final losers play for 3rd and earlier losers share a place. Matches go onto free courts by themselves, in the order they become ready, with the final last. Correct any result and the bracket follows: a match not yet started is made again with the right teams; if a later match has already started you choose to replay it or keep it as played. The bracket is live for the coordinator and spectators, and finishing computes the final places and records each player's placing.
- **Watching** (`/cv/{token}` for one court, `/e/{token}` for the whole event): live scores, results and final positions, with no account and no way to change a score.

Event matches show in history and on player pages as "Sunday cup · Match 1".

## Watch app

`watch/` is a Wear OS 3+ app (built for a Galaxy Watch) that scores a match from the wrist. Pick a live match, tap a half of the screen to give that side the point, and hold Undo to take one back. It signs in with a code, so no password is typed on the watch.

**Build.** Needs JDK 21 and the Android SDK:

```sh
cd watch && ./gradlew :app:assembleDebug
```

The APK is `watch/app/build/outputs/apk/debug/app-debug.apk`. It has no server built in: on first launch it opens Server settings and asks for one. On an emulator, a server on the same machine is `http://10.0.2.2:8080`.

**CI.** The `Watch` workflow (`.github/workflows/watch.yml`) runs the tests and lint, and uploads the APK as the `padel-score-watch-debug-<sha>` artifact. It runs on pushes to `main` and on pull requests only when `watch/`, the web fixtures it tests against or the workflow itself change; every `v*` tag builds. Once the signing secrets below are set, builds on pushes to `main` and on tags are signed with the owner's key; until then, and on pull requests, they use a throwaway debug key, so a PR build cannot update an install made from `main`.

**Signing setup, once.** Make a keystore and add four repository secrets:

```sh
keytool -genkeypair -v -keystore padel-watch.jks -alias padel-watch -keyalg RSA -keysize 4096 -validity 10000
```

```sh
base64 -w0 padel-watch.jks
```

Secrets: `WATCH_KEYSTORE_B64` (the output above), `WATCH_KEYSTORE_PASSWORD`, `WATCH_KEY_ALIAS` (`padel-watch`) and `WATCH_KEY_PASSWORD`. `keytool` makes a PKCS12 keystore, which has no separate key password, so `WATCH_KEY_PASSWORD` must be the same as `WATCH_KEYSTORE_PASSWORD`, or the signed build fails. Keep `padel-watch.jks` somewhere safe and never commit it. Losing it means one reinstall of the app and a new sign-in, nothing worse.

**Sideload.** On the watch, turn on Developer options, then Wi-Fi debugging. Then from a computer on the same network:

```sh
adb pair <ip>:<port>
```

```sh
adb connect <ip>:<port>
```

```sh
adb install -r app-debug.apk
```

**Sign in.** The watch shows a code. On the phone, while signed in, open `<server>/link` and enter it. The watch needs the server reachable from its own network: the paired phone's connection (including its VPN) or the watch's own Wi-Fi. Over the phone's Bluetooth link the watch gets DNS servers that only know the server's public address, so the app falls back to the LAN addresses set under Server settings when DNS gives no LAN answer; the certificate is still checked against the host name.

**Server settings.** The server URL and the LAN addresses are set on the watch, never in the build: the first launch opens Server settings, and afterwards they are under Server settings on the sign-in screen or at the bottom of the match list. Set the server first, then its LAN addresses. Changing the server signs the watch out and clears the LAN addresses; changing only the LAN addresses keeps the sign-in.

**Signed out.** The watch returns to the sign-in screen when its device is revoked on the web Account page, when the user's password is changed, an admin resets it or the account is disabled (all device tokens are revoked), or when you pick "Sign out" in the watch's list. That last item only forgets the sign-in on the watch; it does not revoke the device on the server.

## Releases

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`. If the tagged commit is on `main` and every CI check passes on it, the workflow builds `ghcr.io/jfms7s/padel-score:vX.Y.Z`
for `linux/amd64` and `linux/arm64` (cross-compiled, no emulation) and pushes it with signed build provenance:

```sh
gh attestation verify oci://ghcr.io/jfms7s/padel-score:vX.Y.Z --repo jfms7s/padel-score
```
