# Security policy

## Scope

Snapshade runs locally and opens no network connections. The attack surface is the files it
reads and writes (settings, saved screenshots), the clipboard, the global hotkey, and the
installer. Screen content never leaves the machine.

## Supported versions

The latest release on the Releases page. Older versions are not patched separately.

## Reporting a vulnerability

Please do not open a public issue for a security problem. Use GitHub's private vulnerability
reporting on this repository (**Security → Report a vulnerability**), or contact the maintainer
through the email address on their GitHub profile.

Include the operating system and version, the Snapshade version, steps to reproduce, and what
an attacker could achieve. You will get an acknowledgement within a week; fixes ship as a new
release with a note in `CHANGELOG.md`.

## Verifying a download

- macOS: the `.dmg` and the app inside it are notarized. `spctl --assess --type execute
  --verbose Snapshade.app` reports `Notarized Developer ID`.
- Windows: the installer, `snapshade.exe` and the uninstaller are code-signed. The Digital
  Signatures tab in the file's Properties shows the publisher and a valid chain.
