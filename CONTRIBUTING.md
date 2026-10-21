# Contributing to Poly

Thanks for wanting to help. Poly is built **clean-room**: whether the project
can be distributed at all depends on where every line of it came from. So
before anything else, read **[CLEAN_ROOM.md](CLEAN_ROOM.md)**. This file
explains how those rules apply to people, and how a contribution gets in.

## 1. Pick a role for each area you work on

Work on a component (one Linux syscall, one Windows DLL, one macOS framework)
is split between two roles. You can take either role, but **never both for the
same component**.

| | Spec writer | Implementer |
|---|---|---|
| Observes the real OS (running programs, tracing, measuring) | yes | **no** |
| Public docs (man pages, Microsoft Learn, Apple docs, format specs) | yes | yes |
| Public SDK headers for constants and struct layouts | yes | yes |
| Reads Linux, Wine, ReactOS, XNU or BSD source *for behaviour* | yes | **no** |
| Writes | spec sheets and tests under `specs/` | code under `kernel/`, `crates/`, `personalities/` |
| Sees held-out tests | only the ones they wrote, and only privately | **never** |

Nobody, in either role, may use decompiled, disassembled or debugged
internals of Microsoft or Apple binaries, or leaked source of any kind, and
nobody copies code from another project into this repository.

**Your history counts too.** If you have studied the source of Wine or ReactOS,
or the internals of Windows binaries, you cannot be an implementer for the
Windows (NT) side, even if you don't look at them while working. The same goes
for Linux kernel or glibc source and the Linux personality, and for XNU source
and the macOS side. You can still write specs and tests for those areas, or
implement in another one. If you are unsure whether something you have seen
rules you out, ask in an issue before you start.

## 2. Claim before you start

The coverage map (`coverage/`) records what is done and who is working on
what. Claim the entry you want so nobody else (human or agent) duplicates it:

```sh
cargo xtask coverage claim <entry-id> --agent <your-github-name>
```

and include the claim in your pull request. Release it if you stop. The ordered
plan is [docs/ROADMAP.md](docs/ROADMAP.md): work on the next unticked
milestone, not one further down.

## 3. Spec writers

Follow [specs/README.md](specs/README.md) and [specs/TEMPLATE.yaml](specs/TEMPLATE.yaml):
a spec sheet describes behaviour (results, errors, their order, edge cases) in
words, never as code, and cites the man page or doc it relies on. Tests are
small, deterministic programs that print what they observe.

**Held-out tests stay private.** About 30% of the tests for each entry are held
out from implementers so that "green" means the behaviour was understood, not
fitted to visible tests. Do not put held-out tests in a pull request. Mark which
of your tests you mean to hold out in the PR description, and the maintainer
moves them to the private held-out store. The hand-off to an implementer is then
recorded in [specs/HANDOFF_LOG.md](specs/HANDOFF_LOG.md).

## 4. Implementers

Work only from the spec sheet, the visible tests, public docs and headers.
Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and follow the rules in
[CLAUDE.md](CLAUDE.md), which apply to people as much as to agents:

- logic that does not touch hardware goes in host-testable crates under
  `crates/` or `personalities/`, with unit tests; the kernel stays thin;
- no third-party crates in the kernel;
- every `unsafe` block gets a `// SAFETY:` comment explaining why it is sound;
- constants and struct layouts from public SDK headers cite the header;
- significant design decisions get an ADR in `docs/adr/`.

Run your visible tests on Poly with `cargo xtask pipeline polyrun` (see
[docs/AGENT_PIPELINE.md](docs/AGENT_PIPELINE.md)). An entry turns green only
when the maintainer's runner has passed it against the real OS with the visible
**and** held-out tests; please don't set coverage statuses to green yourself.

## 5. Pull requests

- Run `cargo xtask ci` before you push; it must pass. GitHub runs the same
  check, and `main` only accepts changes that pass it.
- Keep a pull request to one entry or one milestone.
- Fill in the clean-room section of the pull request template.
- Sign off every commit (`git commit -s`), certifying the
  [Developer Certificate of Origin](https://developercertificate.org/): you
  wrote the change, or have the right to submit it under the project's
  licence.
- Don't hand-edit the README progress section or `docs/progress/`; run
  `cargo xtask coverage readme` after changing `coverage/` or the roadmap.

By contributing you agree that your contribution is licensed under the
[Apache License 2.0](LICENSE), like the rest of the project.

## 6. AI agents

Much of Poly is written by AI coding agents. If you use one, it is bound by the
same rules: give it [CLAUDE.md](CLAUDE.md) and [CLEAN_ROOM.md](CLEAN_ROOM.md),
give an implementer agent only the spec sheet and visible tests, and never let
one agent session both observe the real OS and implement the same component.
You are responsible for what your agent submits.

## Questions

Open an issue. For anything about the clean-room rules, ask **before** writing
code: a contribution that can't be shown to be clean has to be removed, however
good it is.
