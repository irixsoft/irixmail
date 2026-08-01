# v0.1.3

## What's new

**Push notifications, overhauled**
- New mail notifications now show the sender and subject, and tapping one opens that message directly
- Notifications fire only for genuinely new incoming mail — reading, flagging, sending, or deleting on another device no longer triggers banners, and dismissed notifications stay dismissed
- Multiple messages arriving together coalesce into a single "N new messages" notification
- The subscription status badge now resolves to "verified" instead of sticking on "verifying"
- Stale unverified subscriptions are cleaned up automatically, and signing out removes the device's push registration on both the browser and the server

**Webmail**
- The compose page no longer crashes in Safari
- Replies and forwards from the compose page keep proper email threading
- The inline reply box under a conversation is replaced with Reply / Reply all / Forward buttons that open the full composer
- Collapsing the folder pane now shows a compact icon rail with tooltips and unread indicators instead of hiding your folders
- Message list polish: avatars and checkboxes are vertically centered, selection checkboxes are clearly visible in both themes, and stray checkboxes no longer appear under avatars on phones

## Upgrading

`sudo irixmail update` on an existing install, or download the binary for your platform below.
