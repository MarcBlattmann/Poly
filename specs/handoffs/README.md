# Hand-off records

One file per hand-off from a spec agent to an implementer agent, written by
`pipeline handoff record` (ADR 0213). Two pull requests that each add a record
add different files, so they never conflict; the legacy table
[`../HANDOFF_LOG.md`](../HANDOFF_LOG.md) is still read, but nothing is added to it.

A file is named `<UTC timestamp>-<spec agent>-<implementer>-<entry hash>.md`
(the hash is the first 12 hex digits of the SHA-256 of the entry id) and holds
the fields of a log row: date (UTC), entry id, spec revision (git), spec agent,
implementer agent, and every file handed over with its SHA-256. Do not edit or
rename one: `pipeline handoff verify` checks the name against the content, and
every hash against git at the recorded revision.
