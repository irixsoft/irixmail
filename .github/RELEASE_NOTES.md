# v0.1.5

## What's new

**Filter rules are back — on our own Sieve engine**
- IRIXMAIL now ships its own RFC 5228 Sieve compiler and interpreter, built from
  scratch for delivery-time filtering: header, address, envelope, exists, and size
  tests, allof/anyof/not, fileinto, redirect, discard, keep, stop, and IMAP flags
- The Filters tab returns to webmail settings: sort incoming mail into folders,
  forward it, mark it read, or discard it, matched on sender, recipient, or subject
- Rules saved before the feature was removed keep working — nothing to migrate
- A broken filter can never lose mail: if a script fails to compile, delivery falls
  back to the inbox

**ManageSieve server**
- A ManageSieve (RFC 5804) server on port 4190 with STARTTLS, so Sieve scripts can
  be managed from external editors and clients
- Scripts edited outside the webmail keep running; the webmail shows them read-only
  with the option to start over with rules
- A `_sieve._tcp` SRV record is included in the generated DNS zone

**Other**
- Filter delivery applies on both direct and relayed inbound mail, and filing into a
  folder still raises a push notification unless the folder is Spam or Trash

## Upgrading

`sudo irixmail update` on an existing install, or download the binary for your platform below. Open port 4190 if you want to reach ManageSieve from outside.
