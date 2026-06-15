# Bottom Player Toolbar (#46) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the player overlay toolbar from the top of the video to the bottom, stacked just above the native `<video controls>` bar, so it no longer covers the picture in windowed playback.

**Architecture:** CSS-only change in the `<style>` block of `templates/base.html`. Three rules change: the toolbar anchor (`top:0` → `bottom:44px`), its scrim gradient direction, and the help-popup anchor (`top:52px` → `bottom:100px`, opening upward). Markup and all JavaScript (`show-controls` toggle, 3 s auto-hide, button handlers) are untouched.

**Tech Stack:** Askama templates, plain CSS. Spec: `docs/superpowers/specs/2026-06-15-bottom-player-toolbar-design.md`.

---

### Task 1: Relocate the toolbar to the bottom

**Files:**
- Modify: `templates/base.html` (the `#player-toolbar`, gradient, and `#player-help` rules at lines ~32–45)

This is a CSS positioning change with no unit-testable behavior (the `oneshot` HTTP harness can't assert rendered CSS geometry). The markup is unchanged, so the existing test suite is the regression guard — it must stay green — and the visual result is confirmed by running the app in Step 4.

- [ ] **Step 1: Confirm the existing suite is green (baseline)**

Run: `cargo test`
Expected: PASS (399 tests; 9 ignored). This is the regression baseline — the markup change in this task must not break it.

- [ ] **Step 2: Edit the three CSS rules in `templates/base.html`**

Find this block (currently around lines 31–35):

```css
    #player-panel{position:relative}
    #player-toolbar{position:absolute;top:0;left:0;right:0;z-index:6;
      display:flex;align-items:center;gap:8px;padding:10px 12px;
      background:linear-gradient(#000,transparent);
      opacity:0;transition:opacity 0.2s;pointer-events:none}
```

Replace it with (anchor to bottom above the native control bar; flip the scrim so it darkens downward):

```css
    #player-panel{position:relative}
    /* bottom:44px / help bottom:100px clear the native <video controls> bar (~30–48px) */
    #player-toolbar{position:absolute;bottom:44px;left:0;right:0;z-index:6;
      display:flex;align-items:center;gap:8px;padding:10px 12px;
      background:linear-gradient(transparent,#000);
      opacity:0;transition:opacity 0.2s;pointer-events:none}
```

Then find the help-popup rule (currently around line 43):

```css
    #player-help{position:absolute;top:52px;right:12px;z-index:7;
```

Replace its anchor so the popup opens upward from above the relocated toolbar:

```css
    #player-help{position:absolute;bottom:100px;right:12px;z-index:7;
```

- [ ] **Step 3: Re-run the suite to confirm no regression**

Run: `cargo test`
Expected: PASS, same counts as Step 1. (Markup is unchanged, so any failure here means an accidental edit outside the CSS rules — revert and redo Step 2.)

- [ ] **Step 4: Manually verify the layout**

Run: `cargo run`, open `http://localhost:3000`, tune a channel, then move the mouse over the player to reveal the controls. Confirm:
- The custom toolbar (`✕ ↑ ↓ … ?`) sits **just above** the native seek/volume bar, not over the top of the picture.
- Both bars appear together on mousemove and hide together after ~3 s idle.
- Clicking `?` opens the shortcuts popup **upward** (above the toolbar), not off the bottom edge.
- Pressing `F` still enters fullscreen with the native controls (unchanged behavior).

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt
cargo clippy -- -D warnings
git add templates/base.html
git commit -m "feat(player): move overlay toolbar to the bottom (#46)"
```

(Note: `cargo fmt`/`clippy` touch only Rust; the change is in a template, so they are run for hygiene/CI parity, not because this edit affects Rust.)

---

### Task 2: Mark idea #46 done

**Files:**
- Modify: `docs/IDEAS.md` (remove the #46 entry from "Open")
- Modify: `docs/CHANGELOG.md` (add #46 with its rationale)

- [ ] **Step 1: Move #46 from `docs/IDEAS.md` Open to `docs/CHANGELOG.md`**

Remove the `46. **Move player overlay buttons to the bottom** …` entry from the `## Open` section of `docs/IDEAS.md`. Add a corresponding completed entry to `docs/CHANGELOG.md` following the file's existing format, summarizing: toolbar moved from `top:0` to `bottom:44px` above the native control bar, scrim flipped, help popup opens upward; CSS-only, fullscreen out of scope. Bump the "N completed ideas" count in `docs/IDEAS.md`'s Done section if present.

- [ ] **Step 2: Commit**

```bash
git add docs/IDEAS.md docs/CHANGELOG.md
git commit -m "docs(ideas): mark #46 done — bottom player toolbar"
```

---

## Self-Review

- **Spec coverage:** Spec changes 1–3 (toolbar anchor, gradient, help popup) → Task 1 Step 2. Testing section (suite stays green + manual visual) → Task 1 Steps 1/3/4. Out-of-scope items (fullscreen, custom controls) are not implemented — correct. Backlog bookkeeping is conventional for this repo → Task 2.
- **Placeholder scan:** No TBD/TODO; every CSS edit shows exact before/after.
- **Type consistency:** No types/signatures involved (CSS + template only).
