# ADR-013: commit-msg hook reads only the `Refs:` trailer

Status: Accepted
Date: 2026-09-23

## Context

Three sources disagreed about how a commit message references a thread:

| Source | Form | Where the hook looks |
|---|---|---|
| SPEC-3.0 §2.5 (3) | `Refs: <id>`, no `@` | the `Refs:` trailer only; scanning the body is forbidden |
| MANUAL / `--help-llm` | `Refs: @<id>` | not stated |
| `hook.rs` (`extract_thread_ids`) | `@<id>` or `KIND-xxxx` | the whole message, subject and body |

Measured against the 3.2.0 hook (`git forum hook check-commit-msg`):

| Message | Result |
|---|---|
| subject `[<id>]` | warning |
| subject `[@<id>]` | accepted |
| trailer `Refs: <id>` (the spec's own form) | warning — never validated |
| trailer `Refs: @<id>` | accepted |
| trailer `Refs: zzzzzzzz` (undefined) | warning only; the spec requires failure |
| body `Thanks @reviewer` | exit 1 — an 8-letter mention is read as a thread ID and the commit is refused |

Two agent sessions independently wrote the ID as a subject tag
(`[7xsi2tjy]`), got the generic warning on every commit, and read it as a
false positive. The warning did not say which form was expected.

## Decision

1. The hook reads thread IDs only from `Refs:` trailers, using Git's own
   trailer parser (`git interpret-trailers --parse --no-divider`) so that
   "trailer" means what it means to Git, including case-insensitive keys and
   folded continuation lines.
2. Each trailer token may be written as `<id>` or `@<id>`; the leading `@` is
   stripped. This departs from SPEC-3.0 §2.5 (3) as first written, which
   allowed only the bare form. The allowance is limited to the trailer: §6
   still defines no display marker and still rejects `@<id>` as CLI input.
3. Trailer tokens without thread-ID shape (`id_alloc::is_valid_thread_id`:
   an 8-character base36 token, or a legacy `KIND-…` ID) are ignored, so a
   `Refs: #123` written for another tracker neither fails nor counts.
4. A thread-ID-shaped token that names no thread fails the commit. Defined
   means a thread ref, a published ref (public-only clones have only those),
   or a migration alias.
5. With no thread ID on a trailer, the hook still exits 0. The warning keeps
   its first line unchanged, for anything that matches on it, and adds the
   expected trailer line, a note that subject tags and body text are not
   read, and the `evidence add` command. When a token elsewhere in the
   message names an existing thread, that ID is filled in.

SPEC-3.0 §2.5 (3) is amended to match.

## Consequences

- Workflows that put `@<id>` in the subject stop being validated and get the
  warning instead; the warning shows the trailer line to use. This is the
  intended behavior change.
- `Refs: <id>` and `Refs: @<id>` both validate, so both the spec's examples
  and the MANUAL's examples work.
- Prose can no longer refuse a commit.
- Each hook run spawns one extra `git interpret-trailers` process. Listing
  the thread refs for the warning's suggestion happens only on the warning
  path.
- The hook still attaches no evidence. The automatic attachment the MANUAL
  described is proposal `o6e7d49a`, which is not implemented; the MANUAL is
  corrected to say so.

## Alternatives

- **Bare IDs only, as SPEC-3.0 was first written.** Rejected: every
  `Refs: @<id>` written from the MANUAL's examples would start warning, and
  the `@` carries no ambiguity inside a trailer.
- **Keep scanning the whole message and change the spec.** Rejected: body
  prose keeps refusing commits (`@reviewer`), and subject tags stay a second
  form that the MANUAL does not describe.
- **Parse `Refs:` lines anywhere in the message.** Rejected: it differs from
  what `git log --format=%(trailers)` reports, which a post-commit
  attachment (`o6e7d49a`) would read.

## Exit criteria

- `tests/hook_test.rs` pins each row of the measurement table above to its
  new result.
- MANUAL, `--help-llm` and SPEC-3.0 §2.5 (3) describe the same form.
