# Security policy

## Supported versions

Security fixes are applied to the latest published Marrow release. Upgrade before reporting an
issue that only affects an older release.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability. Email Mutaqin Aryawijaya at
`mutaqin.aryawijaya@gmail.com` with the affected version, reproduction steps, impact, and any
suggested mitigation. You should receive an acknowledgement within three business days.

## Trust model

Local Marrow stores trust the user account that owns the project files. The dashboard binds to
loopback and rejects browser writes from foreign origins, but other processes running as the same
user can read or modify local files.

The optional backbone is a self-hosted beta. It uses one bearer token and separates spaces by a
validated directory slug. Run it behind HTTPS, use a unique high-entropy token, restrict network
access where possible, and back up the mounted data volume. It does not yet provide per-user roles,
tenant billing boundaries, quotas, or managed recovery.

Release archives include SHA-256 checksums and GitHub artifact attestations. The install script
rejects unverified releases unless the user explicitly opts into legacy behavior.
