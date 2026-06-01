# self-review and pr-bug-review Skills Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the existing `library-review` skill with an extended `self-review` skill (13 universal checks, no language-specific sections), and create a new `pr-bug-review` skill that dispatches three parallel Claude subagents and synthesises a ranked bug report to the console.

**Architecture:** Both skills are standalone markdown instruction files in `~/.claude/skills/`. `self-review` is a direct rewrite of `library-review` — same structure, updated checks. `pr-bug-review` is a new orchestrator skill: it constructs git diff context, dispatches three focused subagents via the Agent tool, waits for results, and passes all three outputs to a synthesis subagent that deduplicates and prints the final report.

**Tech Stack:** Claude Code skills (markdown instruction files), `git diff`, `gh` CLI (optional, not required), Agent tool for subagent dispatch.

---

## File Map

| Action | Path |
|--------|------|
| Create | `~/.claude/skills/self-review.md` |
| Delete | `~/.claude/skills/library-review.md` |
| Create | `~/.claude/skills/pr-bug-review.md` |

---

## Task 1: Create `self-review` skill

**Files:**
- Create: `~/.claude/skills/self-review.md`
- Delete: `~/.claude/skills/library-review.md`

- [ ] **Step 1: Write `self-review.md`**

Create `~/.claude/skills/self-review.md` with this exact content:

```markdown
Detect the primary language(s) in the project or module the user specifies, then apply all checks below.

For each check: ✅ (passes), ⚠️ (partial), or ❌ (fails), with a specific file/line callout when the check fails or warns.

---

## Step 1 — Detect language

Scan file extensions and build/config files:
- `.scala` + `build.sbt` / `project/` → **Scala**
- `.ts`/`.tsx` + `package.json` → **TypeScript**
- `.js`/`.jsx` + `package.json` → **JavaScript**
- `.py` + `pyproject.toml`/`setup.py`/`requirements.txt` → **Python**
- `.java` + `pom.xml`/`build.gradle` → **Java**
- `.kt` + `build.gradle.kts` → **Kotlin**
- `.go` + `go.mod` → **Go**
- `.rs` + `Cargo.toml` → **Rust**

Report detected languages before running checks.

---

## Universal Checks (all languages)

**U1. Minimal public API surface**
Are there methods, types, or fields exposed publicly that should be internal? Flag internal helpers that are `public` by accident and any type that leaks implementation detail through a public interface.

**U2. Dependencies justified**
For every declared dependency (build.sbt, package.json, requirements.txt, go.mod, Cargo.toml, pom.xml, etc.): is it used in more than one place, and does it do something non-trivial to inline? Flag dependencies used in only one method or for a single call site.

**U3. Errors propagated, not swallowed**
Search for silent failure patterns: catching exceptions and returning a default/null/zero value, empty catch blocks, and parse/decode helpers that return a sentinel on failure. Each should surface the error to the caller.

**U4. Invariants enforced at construction**
For types with constrained fields (length, range, format), are constraints validated before or during construction? Flag types that accept arbitrary input with no validation.

**U5. Public API documented with examples**
Every public function, class, or module should have a doc comment (Scaladoc, JSDoc, docstring, Javadoc, godoc, etc.) with: a one-line description, parameter descriptions, return value, and at least one usage example. Flag missing or stub comments.

**U6. Module documented in project overview**
Check the top-level README (or equivalent) for an entry covering this module: a short description, a usage snippet, and navigation to detailed docs. Flag if absent.

**U7. No hardcoded secrets or environment-specific values**
Flag any literal that looks like a token, password, environment-specific URL, or numeric ID that should be configuration. Grep for: `password`, `secret`, `token`, `api_key`, `localhost`, numeric IDs in non-test code.

**U8. KISS/DRY violations**
Grep for near-duplicate function bodies (>5 lines repeated across files). Flag helpers with only one call site. Flag deep nesting (>3 levels) where a guard clause would flatten the logic. Flag abstractions whose only consumer is a single caller.

**U9. HTML accessibility**
Scan all template/HTML files for:
- `<button>` or `<a>` without visible text, `aria-label`, or `aria-labelledby`
- `<input>` without an associated `<label>` (by `for`/`id` pairing) or `aria-label`
- `<img>` without `alt` attribute
- Interactive `<div>` or `<span>` (with `onclick` or `tabindex`) missing `role`
- Landmark regions (navigation, main content, footer) implemented with generic `<div>` instead of semantic elements (`<nav>`, `<main>`, `<footer>`)

**U10. SQL indexes**
Parse migration and schema files for:
- Columns declared with `REFERENCES` (foreign keys) that lack a `CREATE INDEX`
- Column names appearing in `WHERE`, `JOIN ON`, or `ORDER BY` clauses in query files that lack a corresponding index
Flag each unindexed column with the migration file and line where it is declared.

**U11. N+1 query patterns**
Grep for database query calls (patterns like `query`, `execute`, `fetch`, `find_by`, `SELECT`) inside loop constructs (`for`, `while`, `each`, `map`, `iter`, `forEach`). Flag each occurrence with file:line. Note: static analysis cannot confirm all cases — flag candidates for human judgment.

**U12. Dead code**
Flag:
- Exported/public functions with no call sites found in non-test files
- Variables assigned but never read
- Commented-out code blocks (>3 consecutive commented lines)
- `TODO`/`FIXME` comments — run `git log --follow -1 --format="%ar" <file>` to estimate age; flag if the file hasn't changed in >30 days

**U13. Test coverage gaps**
Cross-reference public functions and HTTP route handlers against test files. Flag any public function or route with no corresponding test by name or path pattern. Does not require a coverage tool — uses textual matching between source and test file names/function names.

---

## Output format

1. **Detected language(s):** list them.
2. For each applicable check: one line — status icon, check ID + name, and (on ❌/⚠️) the specific `file:line` and what to fix.
3. **Summary:** `N/13 checks passed` and one sentence naming the most critical issue found.
```

- [ ] **Step 2: Invoke the skill on the MyTV project to verify it runs**

In Claude Code, run:
```
/self-review
```
Expected: the skill runs without error, detects Rust, outputs results for all 13 checks (some ✅, some ⚠️ or ❌), and prints a summary line. The skill should not error out or truncate mid-run.

- [ ] **Step 3: Verify output format matches spec**

Check the console output:
- First line names the detected language(s)
- Each check appears as `✅ U1. ...` or `❌ U8. src/foo.rs:42 — ...`
- Last line is `N/13 checks passed` followed by one sentence
- No language-specific checks appear (no Scala S1–S4, TypeScript T1–T4, etc.)

If anything is missing or malformatted, edit `~/.claude/skills/self-review.md` to fix the instruction wording and re-run.

- [ ] **Step 4: Delete `library-review.md`**

```bash
rm ~/.claude/skills/library-review.md
```

---

## Task 2: Create `pr-bug-review` skill

**Files:**
- Create: `~/.claude/skills/pr-bug-review.md`

- [ ] **Step 1: Write `pr-bug-review.md`**

Create `~/.claude/skills/pr-bug-review.md` with this exact content:

```markdown
## Setup

Determine the diff range:
- If the user passed a branch name or PR number, use that as HEAD and `main` as base.
- If no argument was given, use the current branch as HEAD and `main` as base.

Run:
```bash
git diff main...HEAD
git diff --stat main...HEAD
```

If the diff is empty, print:
```
No changes detected between <HEAD> and main. Nothing to review.
```
and stop.

Save the diff output to use in subagent prompts below. Do NOT pass your full conversation history to subagents — construct their context explicitly.

---

## Phase 1 — Parallel Review

Dispatch all three subagents simultaneously using the Agent tool (run them in a single message with three tool calls). Each subagent receives the diff and a single-lens mandate.

### Correctness Agent prompt

```
You are a Correctness Reviewer. Your job is to find logic bugs in this code diff.

## Diff to review

<paste full git diff here>

## Changed files

<paste git diff --stat output here>

## Your mandate

Find bugs caused by incorrect logic. Look for:
- Off-by-one errors in loops, slices, or index calculations
- Wrong conditional direction (< vs <=, negation errors)
- Missing error propagation (error returned but not checked by caller)
- Incorrect state transitions (e.g. writing to a closed resource)
- Wrong return values (returning the wrong variable, early return before mutation)
- Data races (shared state accessed without synchronisation)

Do NOT comment on security, architecture, or code style — those are covered by other reviewers.

## Output format

One finding per line, sorted by severity:

CRITICAL | file:line | one-sentence description of the bug
HIGH     | file:line | one-sentence description
MEDIUM   | file:line | one-sentence description
LOW      | file:line | one-sentence description

If you find nothing at a given severity level, omit that level entirely.
Return ONLY the findings list. No preamble, no summary.
```

### Security Agent prompt

```
You are a Security Reviewer. Your job is to find security vulnerabilities in this code diff.

## Diff to review

<paste full git diff here>

## Changed files

<paste git diff --stat output here>

## Your mandate

Find security vulnerabilities. Look for:
- SQL injection: user input concatenated into queries without parameterisation
- Command injection: user input passed to shell commands
- Authentication bypass: missing auth checks on protected routes
- Secrets committed: API keys, passwords, tokens hardcoded in source
- Unvalidated input: user-controlled values used without bounds/format checks
- Path traversal: user input used to construct file paths
- Insecure defaults: debug modes, permissive CORS, disabled TLS verification

Do NOT comment on logic correctness, architecture, or code style.

## Output format

One finding per line, sorted by severity:

CRITICAL | file:line | one-sentence description of the vulnerability
HIGH     | file:line | one-sentence description
MEDIUM   | file:line | one-sentence description
LOW      | file:line | one-sentence description

If you find nothing at a given severity level, omit that level entirely.
Return ONLY the findings list. No preamble, no summary.
```

### Architecture Agent prompt

```
You are an Architecture Reviewer. Your job is to find design problems in this code diff.

## Diff to review

<paste full git diff here>

## Changed files

<paste git diff --stat output here>

## Your mandate

Find design and structure problems. Look for:
- DRY violations: logic duplicated across two or more locations in the diff
- SRP breaks: a function or module that now does more than one thing
- Leaky abstractions: internal implementation details exposed through a public interface
- Tight coupling: a module that now directly depends on internals of another
- Missing tests: changed behaviour with no corresponding test change
- Unclear naming: identifiers that don't communicate intent

Do NOT comment on logic correctness or security vulnerabilities.

## Output format

One finding per line, sorted by severity:

CRITICAL | file:line | one-sentence description
HIGH     | file:line | one-sentence description
MEDIUM   | file:line | one-sentence description
LOW      | file:line | one-sentence description

If you find nothing at a given severity level, omit that level entirely.
Return ONLY the findings list. No preamble, no summary.
```

---

## Phase 2 — Synthesis

Once all three subagents have returned, dispatch one synthesis subagent with this prompt:

```
You are a Synthesis Reviewer. You have received bug reports from three independent code reviewers (Correctness, Security, Architecture). Your job is to produce one clean, deduplicated report.

## Correctness findings

<paste Correctness agent output here>

## Security findings

<paste Security agent output here>

## Architecture findings

<paste Architecture agent output here>

## Diff for verification

<paste full git diff here>

## Your steps

1. **Deduplicate:** If two or more agents flagged the same file:line, merge them into one entry. Keep the highest severity. Note all source agents in brackets, e.g. `[Correctness + Security]`.

2. **Verify CRITICAL and HIGH:** For each finding at CRITICAL or HIGH severity, read the relevant lines in the diff to confirm the bug is real. If a finding is a false positive, move it to the "Ruled out" section with a one-sentence reason.

3. **Print the consolidated report** in this exact format:

=== PR Bug Review: <HEAD branch> vs main ===

CRITICAL (N)
  [Agent(s)] file:line — description

HIGH (N)
  [Agent(s)] file:line — description

MEDIUM (N)
  [Agent(s)] file:line — description

LOW (N)
  [Agent(s)] file:line — description

Ruled out (N)
  file:line — flagged by <Agent> as <X>; not a real issue because <Y>

If a severity level has zero findings, omit it entirely.
Return ONLY the report. No preamble.
```

Print the synthesis subagent's output directly to the console.

---

## Error handling

- If any Phase 1 subagent fails or returns empty output, note the missing lens in the synthesis prompt: "Architecture findings: (agent failed — lens missing from this review)". Continue with remaining results.
- If all three subagents fail, print: "All review agents failed. Check your git diff range and try again."
```

- [ ] **Step 2: Invoke the skill on the current branch to verify it runs**

Make sure you are on a branch with at least one commit ahead of `main`, then run:
```
/pr-bug-review
```
Expected: the skill reads the diff, dispatches three subagents (you will see three Agent tool calls fire in parallel), waits for results, dispatches the synthesis subagent, and prints a formatted report ending with the `=== PR Bug Review: ... ===` header and at least one severity section.

- [ ] **Step 3: Verify output format matches spec**

Check the console output:
- Header line: `=== PR Bug Review: <branch> vs main ===`
- At least one of: CRITICAL, HIGH, MEDIUM, LOW, or Ruled out sections
- Each finding has `[Agent]` tag, `file:line`, and a one-sentence description
- No raw subagent output leaks through (no preamble, no "As a security reviewer...")

If the format is wrong, edit the synthesis subagent prompt in `~/.claude/skills/pr-bug-review.md` to tighten the output instructions and re-run.

- [ ] **Step 4: Test empty-diff guard**

Check out `main` and run:
```bash
git checkout main
```
Then run:
```
/pr-bug-review
```
Expected output:
```
No changes detected between main and main. Nothing to review.
```

---

## Post-implementation

After both tasks are complete, update `docs/IDEAS.md` in the MyTV repo to mark ideas #5 and #6 as done:

```bash
# In docs/IDEAS.md, move items 5 and 6 to the done list with ~~strikethrough~~
git -C ~/Workspace/playground/MyTV add docs/IDEAS.md
git -C ~/Workspace/playground/MyTV commit -m "docs: mark ideas #5 and #6 as done"
```
