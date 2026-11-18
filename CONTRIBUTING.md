# Contributing to Termust

Send issues and pull requests to [the Termust fork](https://github.com/tarcisiomiranda/alacritty).
Include the problem, the expected behavior and a short way to reproduce it.
Screenshots help with interface changes.

AI-assisted changes are welcome. You are responsible for understanding,
reviewing and testing the code you submit. State how you tested the change.

Before sending a change, run the relevant checks through mise:

```sh
mise exec -- cargo test --workspace
mise run test:shortcuts
mise run test:install
```

Use the existing Rust style and keep changes focused. Update the YAML spec in
`specs/` and the matching guide when behavior changes. Add a regression test for
bugs that could return.

Termust is an independent fork of Alacritty. Its native tabs, panes and settings
are maintained here. The [original upstream contribution guide](docs/upstream-contributing.md)
is kept as a historical reference; contributions to upstream follow its own rules.
