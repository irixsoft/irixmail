<div align="center">

# IRIXMAIL

**Your own mail server, in one binary.**

SMTP · IMAP · POP3 · JMAP · Webmail · Admin panel — no Postfix, no Dovecot, no glue.

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](LICENSE)
[![Rust 1.94](https://img.shields.io/badge/Rust-1.94-orange.svg)](rust-toolchain.toml)
[![Platform: Linux](https://img.shields.io/badge/Platform-Linux%20x86__64%20%7C%20arm64-lightgrey.svg)](https://github.com/irixsoft/irixmail/releases)
[![Release](https://img.shields.io/github/v/release/irixsoft/irixmail?include_prereleases)](https://github.com/irixsoft/irixmail/releases)

[Install](#install) · [Webmail](#webmail) · [Features](#features) · [CLI](#cli-reference)

</div>

---

Running your own email has always meant assembling a stack: Postfix for SMTP, Dovecot
for IMAP, Rspamd for filtering, OpenDKIM for signing, certbot for certificates — five
config languages, five log formats, five ways to break at 3am.

IRIXMAIL is all of it in a single Rust binary. You run one install command, answer a
few questions, and you have a working mail server with a web admin panel and a
complete webmail client. Nothing else to install.

Published by [irixsoft.com](https://irixsoft.com).

## Install

On a fresh Linux server (x86_64 or arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/irixsoft/irixmail/main/install.sh | sudo sh
```

The installer downloads the latest release, verifies its checksum, creates the
`irixmail` system user, installs a systemd unit, and walks you through setup:
hostname, admin account, DNS records, and a guided Let's Encrypt certificate.

Then enable it on boot:

```sh
sudo systemctl enable --now irixmail
```

To pin a version, pass `IRIXMAIL_VERSION`:

```sh
curl -fsSL https://raw.githubusercontent.com/irixsoft/irixmail/main/install.sh | sudo IRIXMAIL_VERSION=v0.1.3 sh
```

### Before you start

| Requirement | Detail |
| --- | --- |
| **A record** | Point `mail.example.com` at the server's public IP. |
| **Reverse DNS (PTR)** | Set it to the same hostname. Ask your provider — this one is non-negotiable for deliverability. |
| **Outbound port 25** | Many hosts block it by default. Check before you begin. |
| **Open ports** | 25, 80, 443, 587, 465, 143, 993, 110, 995 |

The remaining DNS records MX, SPF, DKIM, DMARC are generated for you during setup
and verified live from the admin panel, so you can see exactly which ones have
propagated and which have not.

## Webmail

IRIXMAIL includes a full webmail client, installable as a progressive web app on a
phone home screen or a desktop dock.

**Mail**

- **Conversation threading** — the message list groups by thread, and a conversation
  opens with the latest message expanded and quoted history collapsed.
- **Full-text search** — a dedicated search page over subjects, message bodies, and
  every address on the envelope.
- **Composer** — rich text or plain text, drag-and-drop attachments with upload
  progress, recipient autocomplete from your contacts, and reply, reply all, and
  forward that quote the original and keep the thread intact.
- **Keyboard shortcuts** — `j`/`k` to move, `Enter` to open, `c` compose, `r`/`a`/`f`
  reply, reply all, forward, `e` archive, `#` delete, `u` read, `s` star, `x` select,
  `/` search, `⌘K` command palette, `?` for the full list.
- **Tags and vacation replies** — colour-coded tags and an autoresponder, configured
  in settings.
- **Remote image blocking** — external images are blocked by default and loaded per
  message on request, with a count of how many were blocked.

**As an app**

- **Installable PWA** — add to the home screen on Android and iOS, or install to the
  dock from any Chromium browser.
- **Push notifications** — Web Push over VAPID, sent by the server with no third party
  relay. New mail raises a notification carrying the sender and subject, and opening it
  goes straight to that message. Other changes sync without a notification.
- **Offline reading** — mail, threads, folders, calendars, and contacts already loaded
  stay readable without a connection, and the message list shows an offline indicator.
  Sending requires a connection; messages are not queued for later delivery.
- **Mobile layout** — a bottom tab bar with a context aware action button, swipe to
  archive or open a message's action sheet, pull to refresh, cozy or compact density,
  a reading pane on the right, at the bottom or off and light, dark or system theme.

**Calendar and contacts**

- Day, week, month, and agenda views, recurring events, drag to reschedule, and
  `.ics` import and export.
- Contacts with photos, groups, `.vcf` import and export, and recent messages from
  each person.
- Both are also served over CalDAV and CardDAV, so Apple Calendar, Thunderbird, and
  other native clients read the same data.

The webmail uses the same JMAP interface exposed to native clients. It signs in to one
account at a time. Multi Account support is planned for the next major release.

## Features

**Mail protocols**

- SMTP inbound on port 25 with full email authentication
- Authenticated submission on 587 (STARTTLS) and 465 (implicit TLS)
- IMAP4rev1 on 143 / 993, including IDLE
- POP3 on 110 / 995
- JMAP (Core + Mail) over HTTPS, for the webmail and native clients alike
- CalDAV and CardDAV, plus JMAP calendars and contacts, from the same store

**Delivery**

- Crash-safe outbound queue with retries and proper bounce handling
- Direct-to-MX delivery, or relay through your own provider (Amazon SES and
  friends) — chosen at setup
- Aliases, forwarding, and domain catch-all
- Vacation auto-responses

**Authentication and anti-spam**

- DKIM signing and verification, SPF, DMARC, and ARC verification
- No machine learning and no training corpus: DNSBL, greylisting, rate limiting,
  policy enforcement, a Spam folder, and manual mark-as-spam
- Argon2id password hashing, optional TOTP, app passwords, and API keys

**Operations**

- Native ACME (Let's Encrypt) issuance and renewal, or bring your own certificate
- Embedded admin panel — accounts, domains, live DNS verification, certificates
- `irixmail backup` and `irixmail restore` as first-class commands
- Self-updating via `irixmail update`, with an optional daily timer

## CLI reference

| Command | Description |
| --- | --- |
| `irixmail run` | Run the server (default; this is what the systemd unit invokes). |
| `irixmail setup` | First-run interactive setup. Resumable if interrupted. |
| `irixmail admin reset-password <email>` | Recover admin access. |
| `irixmail admin api-key create\|list\|revoke` | Manage admin API keys from the shell. |
| `irixmail backup <dir>` | Write a consistent backup set. |
| `irixmail restore <dir>` | Restore from a backup set. |
| `irixmail cert status` | Show certificate state and expiry. |
| `irixmail cert reissue` | Force a certificate reissue. |
| `irixmail update` | Update to the latest release. Add `--check` to look without applying. |

### Ports

| Port | Use |
| --- | --- |
| 25 | SMTP (MX, inbound) |
| 587 | Submission (STARTTLS) |
| 465 | Submission (implicit TLS) |
| 143 / 993 | IMAP (STARTTLS / implicit) |
| 110 / 995 | POP3 (STARTTLS / implicit) |
| 443 | HTTPS — admin panel, webmail, JMAP |
| 80 | HTTP — ACME HTTP-01 challenge, redirect to 443 |

## Repository layout

```
irixmail/
├─ Cargo.toml         # workspace
├─ crates/
│  ├─ irixmail/       # main binary: boot, listeners, CLI subcommands
│  ├─ core/           # shared server state, config, IDs, errors, tracing
│  ├─ store/          # RocksDB store + filesystem blob store + FTS
│  ├─ directory/      # principals, credentials, Argon2id, TOTP, authz
│  ├─ mail/           # message model, ingest, Sieve, delivery
│  ├─ smtp/           # inbound, submission, outbound queue
│  ├─ imap/           # IMAP4rev1 server
│  ├─ pop3/           # POP3 server
│  ├─ jmap/           # JMAP Core + Mail server
│  ├─ dav/            # CalDAV + CardDAV for native calendar and contact clients
│  ├─ http/           # admin REST, JMAP mount, static assets, .well-known
│  ├─ tls/            # rustls resolver + native ACME + cert store
│  └─ dns/            # resolver, record generation, live verification
└─ frontend/
   ├─ admin/          # admin panel (React + Vite + Tailwind)
   └─ webmail/        # webmail PWA (React + Vite + Tailwind)
```

Both frontends are compiled and embedded into the binary, so a deployed IRIXMAIL has
no static file directory to serve or keep in sync.

## Who it's for

IRIXMAIL targets individuals self-hosting personal mail and small teams running their
own mail on a single box. It is built to be understood and operated by one person.

It is not currently aimed at multi-tenant hosting providers or deployments spanning
multiple nodes.

## Status

IRIXMAIL is in early release. Static Linux binaries for x86_64 and arm64 ship on the
[releases page](https://github.com/irixsoft/irixmail/releases), and the installer
always fetches the latest.

Early release means the protocol surface is implemented and working, but it has not
yet accumulated years of production hours across many deployments. If you run it,
keep backups, and open an issue when something breaks.

## Roadmap

The next major release adds multi-account webmail: several mailboxes signed in at
once, with per-account notifications and settings.

Releases before it are bug fixes and polish rather than new protocol surface, native client compatibility, PWA update behaviour, and reported issues.

## Contributing

Issues and pull requests are welcome. Bug reports are most useful with the IRIXMAIL
version, the Linux distribution, and the relevant log output.

Pull requests require agreement to the [Contributor License Agreement](CLA.md). It is a
one-time agreement, stated in your first pull request. You keep the copyright in your
contribution; IRIXSOFT LTD receives the rights needed to distribute and license IRIXMAIL as
a whole.

## Trademark

IRIXMAIL and the IRIXMAIL logo are trademarks of IRIXSOFT LTD. The license below covers the
source code and grants no rights to the name or the logo.

Forks and modified versions may not use the IRIXMAIL name or logo in a way that suggests
they are the official project or endorsed by IRIXSOFT LTD. Naming your fork something else
is the simplest way to stay clear of this.

## License

AGPL-3.0-only. Copyright © 2026 IRIXSOFT LTD. See [LICENSE](LICENSE) for the full text.
Licenses of the third-party dependencies bundled into the binary are listed in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md), regenerated for every release.

In practice: run IRIXMAIL for yourself or your organisation with no obligations. If
you modify it and offer the modified version to others over a network, make your
changes available under the same license.
