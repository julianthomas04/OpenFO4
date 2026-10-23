# OpenFO4 development guidelines

## In addition to this file, also read AGENTS.local.md (if present), as well as README.md and CONTRIBUTING.md.

# Code Quality

## 1. Readability and Maintainability

- Keep comments to a minimum.
- Code should be **self-documenting** at all times.
- Follow the **DRY (Don't Repeat Yourself)** principle.
- The code should be readable and structured sensibly.
- Rely on proven, industry standard approaches to problems where applicable.
- The code should be testable.
- Use sensible defensive programming.

## 2. Separation of Concerns

Maintain a clear separation of concerns at every architectural level:

- Functions
- Classes
- Files
- Folders

Do not hesitate to restructure code or reorganize files to maintain these principles.

## 3. Error Handling and Correctness

- Implement robust error handling.
- **Never silently swallow errors.**
- Handle different error types differently when appropriate.
- Avoid grouping unrelated error types into generic handlers.
- Account for all possible execution paths.
- Never allow undefined behavior or ambiguity.

# Git
- Follow the conventional commits format for commits (type: description): https://www.conventionalcommits.org
- Avoid committing temporary files and changes meant for experimentation, prototyping, etc.
- Only update the .gitignore if you have a good reason to do so.

# Legal
- NEVER commit a game asset, neither original, nor converted. Monitor the git history closely, should an asset slip through, rewrite the history to remove it again and then take precautions to avoid this happening again in the future (including in future sessions with a clean context window).
- Decompilations is strictly forbidden. Rely on public information, as well as observations to map behavior.
