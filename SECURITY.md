# Security policy

## Reporting a vulnerability

Please do not report security problems in public issues, pull requests or discussions.

Report them privately through GitHub:
[Report a vulnerability](https://github.com/Rhombicdodecahedron/hive/security/advisories/new)
(repository **Security** tab → **Report a vulnerability**). Only the maintainers can see the
report.

Include what you can of:

- the version (`hive --version`) and platform, and whether `hive serve` or `hive mcp` is
  involved;
- steps to reproduce, or a proof of concept;
- the impact you expect, and any fix you suggest.

We will acknowledge the report, keep you updated in the advisory, and agree on a disclosure
date with you. Fixes are released as a new version, and the advisory is published once users
can update.

## What counts

Hive can post on the TikTok accounts logged in on the phones it drives, so anything that lets
someone else make it post, read files or run commands is in scope: for example a way to reach
`hive serve` from another machine, a request that makes it read or push a file it shouldn't,
or text in a post that ends up run by the phone's shell.

`hive serve` listens on 127.0.0.1 by default. Exposing it on a network is a configuration you
choose and isn't a vulnerability by itself.

## Supported versions

Security fixes go into the latest release. Please check that the problem still occurs there
before reporting.
