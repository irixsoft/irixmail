# v0.1.6

## What's new

**Sign-ins that survive restarts**
- Sessions are stored in the database, so the daily auto-update restart no longer
  logs you out. The webmail stays signed in through 90 days of inactivity and renews
  itself while you use it; the admin panel signs out after 12 idle hours
- Changing or resetting a password signs that account out everywhere

**Admin panel and webmail are separate**
- Each has its own sign-in. Being signed in to the webmail no longer affects `/admin`,
  and a non-admin account is refused at the admin login instead of landing on a
  sign-out screen
- Admin sessions cannot read mail and webmail sessions cannot reach admin routes,
  even for administrator accounts

**Several accounts in the webmail**
- Add more mailboxes from this server and switch between them from the avatar menu
  or the new Accounts section in settings
- Each account keeps its own offline cache and notification setting on the device
- New-mail notifications name the account when more than one is signed in, and open
  that account when tapped

**Backup and restore**
- Settings in the admin panel has a Download backup button that streams a `.tar.gz`
  of the whole server while it runs: accounts, mail, settings, DKIM keys, the
  credential key and the TLS certificate
- `irixmail setup` opens with a fresh-or-restore choice. Point it at the archive on a
  new host and the hostname, relay, accounts and certificate come back; admin
  creation is skipped when the archive already has one
- `irixmail backup <file.tar.gz>` and `irixmail restore <file.tar.gz>` write and read
  the same archive from the shell

## Upgrading

`sudo irixmail update` on an existing install, or download the binary for your platform below. Everyone is signed out once after this update, because sessions moved into the database. Backup directories written by earlier versions cannot be restored by this one; take a fresh archive after upgrading.
