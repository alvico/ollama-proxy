# Working conventions

## Commits

- Stage only the files touched by the current change (list them explicitly, e.g. `git add README.md start-stack.ps1`). Never use `git add -A` or `git add .`.
- Before committing, review `git status` and `git diff` to confirm nothing unrelated is included.
- Leave unrelated working-tree changes (e.g. `config.json` tuning tweaks) unstaged unless the user asks for them to be committed.
