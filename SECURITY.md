# Security policy

## Supported versions

The latest commit on `main` is the supported version.

## Reporting a vulnerability

Do not open a public issue for exploitable problems. Report them privately via
GitHub Security Advisories:

<https://github.com/ChasLui/osview-rs/security/advisories/new>

Include the command you ran, the tool version/commit, and a minimal description
of the crafted input (PLY/NPY layout) when the issue is a parser defect.

`osview-rs` parses untrusted files locally and writes a PNG and a JSON report;
it makes no network calls. Parser panics, unbounded allocation and invalid JSON
output are considered bugs and are handled with the normal issue tracker once
the report is public.
