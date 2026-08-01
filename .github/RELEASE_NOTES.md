# v0.1.4

## What's new

**Search actually searches**
- Combining more than one search filter no longer returns an empty list — the server now understands AND, OR, and NOT filter groups
- Filtering by attachment works: arriving mail is marked as carrying an attachment, and existing mail is marked once on the first start after upgrading
- The contacts pane shows recent mail from a person again

**Branding**
- The real IRIXMAIL logo replaces the placeholder wordmark in the webmail, the admin panel, and both sign-in pages
- New app icon and favicon everywhere, including the installed PWA and push notifications

**Updates you can see**
- After an update, an open webmail tab now offers a Refresh prompt instead of silently breaking on the old assets

**Other**
- The webmail settings page links to the source code, as the AGPL requires

## Upgrading

`sudo irixmail update` on an existing install, or download the binary for your platform below.
