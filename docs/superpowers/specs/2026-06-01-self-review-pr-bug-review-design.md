# Design: `self-review` and `pr-bug-review` Skills

**Date:** 2026-06-01
**Status:** Approved

---

## Overview

Two new general-purpose Claude Code skills for code quality review and PR bug detection. Both are reusable across any project, not MyTV-specific.

---

## Skill 1: `self-review`

### Purpose

A self-check skill the developer runs before committing or opening a PR. Replaces and extends the existing `library-review` skill. Language-specific check sections are removed — the skill focuses on universally applicable quality signals.

### File

`~/.claude/skills/self-review.md`

### Scope change from `library-review`

- **Keep:** U1–U7 (universal checks), language detection step, output format
- **Remove:** all language-specific check sections (Scala, TypeScript, Python, Java/Kotlin, Go, Rust)
- **Add:** U8–U13 (six new universal checks below)

### New Checks

**U8. KISS/DRY violations**
Grep for near-duplicate function bodies (>5 lines repeated across files). Flag helpers with only one call site. Flag deep nesting (>3 levels) where a guard clause would flatten the logic. Flag abstractions whose only consumer is a single caller.

**U9. HTML accessibility**
Scan all template/HTML files for:
- `<button>` or `<a>` without visible text, `aria-label`, or `aria-labelledby`
- `<input>` without an associated `<label>` (by `for`/`id` pairing) or `aria-label`
- `<img>` without `alt` attribute
- Interactive `<div>` or `<span>` (with `onclick` or `tabindex`) missing `role`
- Landmark regions (navigation, main content, footer) implemented with generic `<div>` instead of semantic elements

**U10. SQL indexes**
Parse migration and schema files for:
- Columns declared with `REFERENCES` (foreign keys) that lack a `CREATE INDEX`
- Column names appearing in `WHERE`, `JOIN ON`, or `ORDER BY` clauses in query files that lack a corresponding index
Flag each unindexed column with the migration file and line where it is declared.

**U11. N+1 query patterns**
Grep for database query calls (patterns like `query`, `execute`, `fetch`, `find_by`) inside loop constructs (`for`, `while`, `each`, `map`, `iter`). Flag each occurrence with file:line. Note: static analysis cannot confirm all cases — flag candidates for human judgment.

**U12. Dead code**
Flag:
- Exported/public functions with no call sites found in non-test files
- Variables assigned but never read
- Commented-out code blocks (>3 consecutive commented lines)
- `TODO`/`FIXME` comments older than 30 days (if git blame is available)

**U13. Test coverage gaps**
Cross-reference public functions and HTTP route handlers against test files. Flag any public function or route with no corresponding test by name or path pattern. Does not require a coverage tool — uses textual matching between source and test file names/function names.

### Output Format

Same as `library-review`:
1. **Detected language(s)**
2. Per check: `✅ / ⚠️ / ❌` — check ID, name, and `file:line` on failures
3. **Summary:** `N/13 checks passed` and the most critical issue found

---

## Skill 2: `pr-bug-review`

### Purpose

A self-check skill that dispatches three parallel Claude subagents against a PR diff, each reviewing through a single lens, then synthesises findings into one ranked console report. Designed for use before pushing or requesting review — not for posting to GitHub.

### File

`~/.claude/skills/pr-bug-review.md`

### Invocation

`/pr-bug-review [PR-number | branch-name]`

If no argument is given, diff current branch against `main`.

### Phase 1 — Parallel Review (3 subagents)

Dispatched simultaneously via the `dispatching-parallel-agents` skill pattern. Each subagent receives:
- The full `git diff` output for the target range
- The list of changed files
- A single-lens mandate — it must not stray into the other lenses

| Agent | Lens | Finds |
|---|---|---|
| **Correctness** | Logic bugs | Off-by-ones, wrong conditionals, missing error propagation, data races, incorrect state transitions, wrong return values |
| **Security** | Vulnerabilities | SQL/command injection, auth bypass, secrets committed, unvalidated user input, insecure defaults, path traversal |
| **Architecture** | Design quality | Coupling, DRY violations, SRP breaks, leaky abstractions, missing tests for changed behaviour, unclear naming |

Each agent outputs findings as a ranked flat list:

```
CRITICAL | file:line | one-sentence description
HIGH     | file:line | one-sentence description
MEDIUM   | file:line | one-sentence description
LOW      | file:line | one-sentence description
```

### Phase 2 — Synthesis (1 subagent)

Receives all three ranked lists. Steps (in order):

1. **Deduplicate** — merge findings that point to the same file:line across agents; keep the highest severity rating and note which agents flagged it
2. **Verify CRITICAL and HIGH** — for each finding at these severities, read the relevant source file section to confirm it is real; mark false positives with a brief reason
3. **Print consolidated report** — findings grouped by severity (CRITICAL → HIGH → MEDIUM → LOW); false positives listed at the end in a separate "Ruled out" section

### Output

Console only. No files written. No GitHub API calls.

```
=== PR Bug Review: <branch> vs <base> ===

CRITICAL (N)
  [Correctness + Security] src/auth.rs:42 — ...

HIGH (N)
  [Architecture] src/routes/admin.rs:88 — ...

MEDIUM (N)
  ...

LOW (N)
  ...

Ruled out (N)
  src/foo.rs:12 — flagged by Correctness as X; not a bug because Y
```

### Error handling

- If the diff is empty, print "No changes detected between <branch> and <base>" and exit.
- If any subagent fails, the synthesis agent notes which lens is missing and continues with the remaining results.

---

## Implementation Notes

- Both skills are general-purpose: no MyTV-specific logic, no hardcoded paths.
- `self-review` replaces `library-review` — the old file should be removed after the new one is verified working.
- `pr-bug-review` uses the `dispatching-parallel-agents` skill pattern for Phase 1.
- The synthesis agent in `pr-bug-review` must be given explicit instructions not to re-review the diff itself — its only job is cross-referencing and verifying the three input reports.
