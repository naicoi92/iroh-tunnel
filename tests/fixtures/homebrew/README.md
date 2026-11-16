# Homebrew artifact contract fixture

`cask/artifact/binary.rb`, `cask/artifact/symlinked.rb` and
`extend/os/cask/artifact/symlinked.rb` are unmodified Homebrew/brew sources at
commit `7d562e1c8f8d233c822eb029c33a6ce1a4a147d6` (BSD-2-Clause; see
`LICENSE.txt`). Their conflict, link and unlink policy executes in the cask
integration tests; no network fetch or Homebrew installation is needed.

`cask/artifact/relocated.rb` is our minimal test boundary: paths, output, type
annotations and unprivileged filesystem utilities. The Rust test's Ruby harness
supplies DSL and installer lifecycle boundaries. Commands run locally; `xattr`
is recorded and can fail on request. The fixture uses Homebrew's generic Unix
link command, not its macOS-specific command/metadata extension. Thus these tests
are not a native Homebrew, privilege, sandbox or metadata test.

When refreshing the pinned sources, review upstream behavior changes and keep
these files unmodified so tests do not merely mirror our own ownership policy.

`legacy-combined.rb` is archived renderer output from project commit
`6ae7f58c7da407fb37c72e09d4e2feff5160475d`, with version `1.2.2` and a fixture
checksum. It exercises the expected migration from the combined cask's old
manual relative alias. The harness models old-uninstall/new-install ordering;
it is not proof that every installed Homebrew version migrates identically.
