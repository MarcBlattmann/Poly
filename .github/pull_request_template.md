## What this changes

<!-- One entry or milestone per pull request. Name the coverage entry or roadmap line. -->

## Clean room

Read [CLEAN_ROOM.md](../CLEAN_ROOM.md) and [CONTRIBUTING.md](../CONTRIBUTING.md) first.

- **My role for this change:** spec writer / implementer <!-- keep one -->
- [ ] I have not used decompiled, disassembled or debugged internals of Microsoft or Apple binaries, or leaked source code.
- [ ] I have not copied code from any other project into this pull request.
- [ ] **Implementers:** I worked only from the spec sheet, the visible tests, public documentation and public SDK headers. I have not observed the real OS for this component, I have not seen its held-out tests, and I have not studied the source of the OS it imitates or of Wine/ReactOS (for Windows), the Linux kernel/glibc (for Linux) or XNU (for macOS).
- [ ] **Spec writers:** the spec describes behaviour in words, not code, and cites the docs it relies on. Tests I want held out are listed below, not included.

<!-- Spec writers: list the tests to hold out here. -->

## Checks

- [ ] `cargo xtask ci` passes locally.
- [ ] Every commit is signed off (`git commit -s`).
- [ ] The coverage entry is claimed in this pull request (`cargo xtask coverage claim`).
