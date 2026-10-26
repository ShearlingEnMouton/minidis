# Security Policy

## Supported versions

Security fixes are made for the latest release only. Older versions are not patched.

## Reporting a vulnerability

Do not open a public issue for a security problem. Report it privately through GitHub's private vulnerability reporting instead: open the **Security** tab of this repository and select **Report a vulnerability**, or go directly to <https://github.com/ShearlingEnMouton/minidis/security/advisories/new>.

A useful report includes:

- The affected version or commit
- Steps to reproduce the problem, ideally with the exact commands sent to the server
- The impact, for example a crash, data loss, or reading data that should not be reachable

minidis is maintained by one person. Reports are handled on a best-effort basis, without a guaranteed response time.

## Scope

minidis has no authentication and no TLS. It is meant to run on a trusted network, for example inside a Kubernetes cluster behind a `ClusterIP` Service. Exposing it to untrusted clients is not supported, and the absence of authentication is not treated as a vulnerability.
