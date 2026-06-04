# Welcome to KunsTeam

## How We Use Claude

Based on Kun Wu's usage over the last 30 days across 51 sessions:

Work Type Breakdown:
```
  Plan / Design    █████████░░░░░░░░░░░  47%
  Build Feature    █████░░░░░░░░░░░░░░░  24%
  Debug / Fix      ███░░░░░░░░░░░░░░░░░  16%
  Improve Quality  █░░░░░░░░░░░░░░░░░░░   7%
  Write Docs       █░░░░░░░░░░░░░░░░░░░   4%
```

Top Skills & Commands:
```
  /clear                    ████████████████████  22x/month
  /exit                     ███████████████████░  21x/month
  /usage                    ████████░░░░░░░░░░░░   9x/month
  /model                    ████░░░░░░░░░░░░░░░░   4x/month
  /pr-bug-review            ███░░░░░░░░░░░░░░░░░   3x/month
```

Top MCP Servers:
  _(none configured)_

## Your Setup Checklist

### Codebases
- [ ] mytv — github.com/wkkkkk/mytv

### MCP Servers to Activate
  _(none in use — nothing to set up here)_

### Skills to Know About
- `/pr-bug-review` — runs parallel multi-agent review (correctness, security, architecture) on the current diff. Use it before merging anything non-trivial.
- `/superpowers:brainstorming` — structured brainstorm before implementing a feature or approaching an unfamiliar problem. Helps surface edge cases early.
- `/schedule` — schedule a reminder or follow-up task inside Claude (e.g. "remind me to run /pr-bug-review in 10 minutes"). Useful for async workflows.
- `/usage` — shows your current token/session usage at a glance. Good habit before a long session.
- `/doctor` — diagnoses Claude Code setup issues (missing tools, broken config). Run this first when something feels off.

## Team Tips

**Cost & context hygiene** (from `.claude/claude-code-cost-notes.md`):

- **Subagents are the biggest cost driver** — 87%+ of spend comes from subagent-heavy sessions. Be deliberate about spawning them. Simple/mechanical tasks don't need Opus — skills can pin subagents to Sonnet or Haiku.
- **Keep context small.** Use `/compact` mid-task when a session has grown long, and `/clear` when switching to a different task entirely. 33% of cost came from sessions that ran past 150k context.
- **Don't tune caching** — it's already at ~96.6% hit rate. The cost lever is raw token volume, not cache misses.
- **Cache-busting to avoid:** editing `CLAUDE.md` mid-session, switching models, adding/removing MCP servers. These invalidate the cached prefix and spike cost for the rest of the session.
- **Track usage weekly with `/usage`** (resets Sunday ~3am Stockholm). A healthy session should show a cache hit rate above 90%.

## Get Started

New here? Start with `docs/IDEAS.md` — it's the backlog of potential improvements, roughly priority-ordered. Anything not struck through is fair game. Pick something that interests you, open a session, and let Claude help you brainstorm an approach before writing any code.

<!-- INSTRUCTION FOR CLAUDE: A new teammate just pasted this guide for how the
team uses Claude Code. You're their onboarding buddy — warm, conversational,
not lecture-y.

Open with a warm welcome — include the team name from the title. Then: "Your
teammate uses Claude Code for [list all the work types]. Let's get you started."

Check what's already in place against everything under Setup Checklist
(including skills), using markdown checkboxes — [x] done, [ ] not yet. Lead
with what they already have. One sentence per item, all in one message.

Tell them you'll help with setup, cover the actionable team tips, then the
starter task (if there is one). Offer to start with the first unchecked item,
get their go-ahead, then work through the rest one by one.

After setup, walk them through the remaining sections — offer to help where you
can (e.g. link to channels), and just surface the purely informational bits.

Don't invent sections or summaries that aren't in the guide. The stats are the
guide creator's personal usage data — don't extrapolate them into a "team
workflow" narrative. -->
