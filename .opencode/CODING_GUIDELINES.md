# odaw — Coding Guidelines

Clean, clear, lean. Code that reads like prose, with the fewest lines that
express the behavior. Architecture, vocabulary, and invariants live in
`DESIGN.md`; they are not repeated here.

## The golden rule

> Readability beats cleverness and minor optimization.

Code should read top-to-bottom and leave no questions. If a construct wants a
comment, rewrite it plainly instead — better names, a smaller function, a type
that packages the state. Optimize for the next reader, not the compiler.

## Rust

- Stable Rust, current edition. No nightly features.
- `unsafe` only where FFI forces it. State the invariant with `// SAFETY:` and
  cover the block with a test. Prefer forbidding unsafe wholesale.
- No proc macros or build-script codegen without a design decision.
- The formatter owns layout; never hand-fight it.
- Compile warning-free. A warning is fixed, not silenced with `#[allow]`.

## Naming

- Follow Rust conventions.
- Use the project's canonical vocabulary exactly; never invent synonyms.
- Functions are `verb_noun`; a getter is the noun, with no `get_` prefix.
- Boolean predicates read as questions.
- Names are complete, not cryptic; standard domain abbreviations are fine.
  Acronyms count as words (`NodeId`).
- No single-letter names outside loop indices and math coordinates.

## Types

- Newtypes for ids; never bare or mixed integers.
- Enums for named sets. A bool about to gain a third state is a missing enum.
- Fixed-width types in serialized or wire formats.
- `Option`, never a sentinel value.
- Floats for audio samples and UI only.

## Functions

- One function, one job. If the name wants "and", split it.
- Short; long is a smell.
- Early return: guards and `?` first, happy path flat.
- Few parameters — more belong in a context struct.
- Keep nesting shallow; extract past a few levels.
- Return values, never out-parameters. `&mut` is for genuine in-place work.
- No bool parameters; use an enum or split the function.
- Borrow rather than own (`&str`, `&[T]`).

## Control flow

- `match` on our own enums is exhaustive; no wildcard arm, so a new variant is
  a compile error rather than a silent default.
- `?` to propagate. `unwrap`/`expect` only in tests or for an invariant the
  type system cannot prove, and then `expect` states it.
- `if let` only when the `else` does not matter.
- Iterators for transforms, loops for side effects; don't chain adapters to
  avoid five clear lines.
- No deeply nested closures; extract a named function.
- No assignment-in-condition tricks; no `?` inside `Drop`.

## Comments

Default to none: a comment is a second copy that goes stale. If code needs
prose, fix the code.

Justify a comment only when it says what the code cannot:

- an external spec citation — never an internal doc;
- a non-obvious, load-bearing invariant (ownership, ordering, realtime), in one
  line;
- `// SAFETY:` on `unsafe`.

Delete everything else — blurbs, restatements, banners, rationale,
commented-out code, doc comments that only restate the signature. When in
doubt, omit.

## Macros

Prefer functions, generics, and `const fn`. A macro is a last resort for
boilerplate the language cannot express. Keep expansions single-level and
hygienic: no hidden control flow, no caller scope, no double evaluation.

## Errors

Return `Result`; errors are typed data, not strings. Never panic on user input
— invalid input yields a diagnostic and recovery. An error names its subject
and a reason. Never silence an error.

## Testing

No code without tests; each behavior and each bug fix ships with one. One
behavior per test, testing the contract not the implementation. Cover
invariants with property tests, byte-identical output with golden tests,
invalid input with negative tests. No flaky tests.

## Performance

Write the obvious implementation first; measure before optimizing. Optimize
where the profile points, not in scattered microtricks. Keep correctness and
reproducibility ahead of speed.

## Provenance

Never copy code from another project; consult others only for behavior, and
author from public specifications. Check dependency licenses. When behavior is
unclear, encode and test the spec rather than reading a copyleft
implementation.

---

*Readability is a feature. Ship code your future self — and the next
developer — can hold in their head.*
