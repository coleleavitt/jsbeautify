# jsbeautify — Claude Code notes

@AGENTS.md

## Claude Code setup

- Hooks (`.claude/settings.json`): edits to `target/` and scratch binaries are
  refused, edited `.rs` files are run through `rustfmt`, and a stop hook runs
  clippy when Rust files changed. It blocks only on build errors.
- Subagents (`.claude/agents/`): run `deobf-soundness-reviewer` after adding or
  changing a pass or renamer, and `hostile-input-reviewer` after changes to
  parsing, decoders, or network fetch code. `codegraph-vuln-analyst` audits a
  subsystem for bugs.
- Skills (`.claude/skills/`): `new-pass` scaffolds a deobfuscation pass, `gate`
  runs the full fmt/clippy/test gate, `vuln-scan` runs a bug hunt.
