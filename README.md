# phone-media-backup-rs (`p2r`)

> Pull photos and videos off your Android phone over SSH, sort them into a dated tree on your server, and verify every byte — incrementally, and without a cloud account.

## What & why

A small Rust binary (`p2r`, ~1k LOC) that backs up the camera roll from a phone running an SSH server (e.g. [SimpleSSHD](https://www.galexander.org/software/simplesshd)) to a directory tree on a Linux host. It's incremental (a server-side marker file tracks the last successful run), content-verified (xxh3 partial + full compare), and EXIF-aware (files are bucketed by capture date, not download date).

Built because the existing options were either cloud-bound (Google Photos), GUI-only (Syncthing-style folder sync without dating), or shell scripts that silently dropped files on failure.

## Features

- 💾 Backup `.jpg`, `.dng`, `.mp4` from a phone over `sshfs` + `ssh2`
- 📅 Bucket files into `YYYY/YYYY-MM-DD/` based on EXIF `DateTime` (jpg/dng) or the `PXL_YYYYMMDD_…` filename pattern (mp4)
- 📍 Incremental: a remote marker file scopes each run to "files newer than last time"
- 🧮 Verify-on-copy: xxh3 of head+tail 64 KiB, then full-file 64 KiB-streamed compare
- 🧹 Optional source-side cleanup: delete phone-side files older than N days
- 🔎 `verify` subcommand: re-walks the phone and reports anything missing on the server
- 🔔 Optional webhook notifier (e.g. Home Assistant) for backup status
- 📊 Progress bars + structured logs via `tracing`

## Architecture

Single binary, five modules:

| Module        | Role                                                              |
| ------------- | ----------------------------------------------------------------- |
| `main.rs`     | CLI entry, dispatches `backup` / `verify` / `clean`               |
| `config.rs`   | `clap` args + serde-validated YAML schema (`deny_unknown_fields`) |
| `ssh.rs`      | `Mount` (RAII sshfs), `Ssh` (libssh2 session), `Phone` trait      |
| `file.rs`     | EXIF/regex date extraction, copy + verify, marker logic           |
| `notifier.rs` | Optional HTTP POST on backup status                               |

### Data flow

```
       ┌──────────────┐         ┌────────────────────┐
       │  phone       │  ssh    │  p2r (host)        │
       │  SimpleSSHD  │◀────────│  read marker file  │
       └──────┬───────┘         └─────────┬──────────┘
              │                           │ "2026-04-30"
              │   find -newermt $marker   │
              │◀──────────────────────────┤
              │   list of new paths       │
              ├──────────────────────────▶│
              │                           │
              │   sshfs mount (FUSE)      │
              │◀═════════════════════════▶│
              │   walk paths, read bytes  │
              │                           │
              │                           ▼
              │              ┌────────────────────────┐
              │              │ resolve_destination    │
              │              │  (EXIF / filename)     │
              │              └─────────┬──────────────┘
              │                        │
              │                        ▼
              │              ┌────────────────────────┐
              │              │ skip if partial-hash   │
              │              │ matches existing dest  │
              │              │ else copy + strict     │
              │              │ compare                │
              │              └─────────┬──────────────┘
              │                        │
              │   write new marker     ▼
              │◀──── (latest_date - 1 day) ──────
              │                        │
              │                        ▼
              │                  notifier?
```

The "latest_date − 1 day" rewind is deliberate: same-day files added after the run still get picked up next time.

## Configuration

Config files are YAML, loaded by `confy`. Pass the **path** with `--config`:

```yaml
# config/myphone.yaml
general:
  mountpoint: /home/user/sshmount/

source:
  user: user
  ip: 192.168.1.3
  port: 2222
  known_hosts: ~/.ssh/known_hosts # optional, default shown
  cert: ~/.ssh/id_ed25519 # optional, default shown
  path: /sdcard/DCIM/Camera # absolute, no shell metachars
  clean:
    older_than_days: 30 # must be > 0

destination:
  path: /mnt/photos/user

# Optional: omit to disable status notifications
notifier:
  url: https://homeassistant.example/api/webhook/backup-status
  token: your-bearer-token-here
```

Unknown keys are a hard error — typos won't be silently ignored. See `config.example` for a copy-pasteable template.

## Build & run

Requires a Linux host with `sshfs` installed.

```bash
cargo build --release
./target/release/p2r --config config/myphone.yaml             # backup (default)
./target/release/p2r --config config/myphone.yaml backup --clean
./target/release/p2r --config config/myphone.yaml verify
./target/release/p2r --config config/myphone.yaml clean -n    # dry-run delete
```

Or via Docker (image bundles `sshfs`, docker expects mountpoint to be `/root/sshmount`):

```bash
docker build -t p2r .
docker run --rm --device /dev/fuse --cap-add SYS_ADMIN \
    -v ~/.ssh/id_rsa:/root/.ssh/id_rsa:ro \
    -v ~/.ssh/known_hosts:/root/.ssh/known_hosts:ro \
    -v /mnt/photos:/mnt/photos \
    -v $(pwd)/config:/config p2r --config /config/myphone
```

Logs are controlled with `RUST_LOG` (default `debug`):

```bash
RUST_LOG=info p2r --config config/myphone.yaml
```

## Security model

- **Trusts the phone within the LAN.** `p2r` assumes the phone's SSH endpoint is reachable on a network you control. It is not designed to traverse hostile networks.
- **Requires SSH host-key pinning.** The first connection writes the phone's host key into `known_hosts` (TOFU); subsequent runs require an exact match and will refuse to connect on mismatch. Configure the file via `source.known_hosts`.
- **Secrets stay out of the repo.** Config files live outside source control. The notifier `token` is read from your YAML, not committed; pubkey auth is done with the file at `source.cert`. There are no embedded credentials in the binary.

### Limitations / threat model

- A compromised phone can serve manipulated EXIF or filenames; `p2r` will faithfully bucket whatever date the phone presents.
- The remote `path` and `marker` filename are validated (absolute, no `..`, no shell metacharacters), but `p2r` does shell out for `sshfs`/`umount` and runs `find` over SSH — review `ssh.rs` before exposing it to untrusted config sources.
- The xxh3 verify is a non-cryptographic integrity check (corruption, not tampering). For tamper-evident archives, layer a signed checksum on top.
- "Only Pixel `PXL_*` MP4 filenames carry a parsable date — other MP4s are skipped.
- No resume on interrupt.

## Project layout

```
src/
  main.rs        # CLI entry
  lib.rs         # run_backup / run_verify / run_clean
  config.rs      # YAML schema + clap args
  ssh.rs         # sshfs mount + libssh2 session + Phone trait
  file.rs        # date resolution, copy, verify, hash
  helpers.rs     # mtime preservation, ext extraction
  notifier.rs    # optional webhook
config/          # local-only YAML configs (gitignored where applicable)
config.example   # template
tests/           # integration tests
```

## TODO

- [ ] Select a specific subdirectory of the source path to backup
- [ ] `info` subcommand (source stats, last backup summary)
- [ ] Support more file types (e.g. `.heic`, `.avi`)

## License

MIT — see [LICENSE](LICENSE).
