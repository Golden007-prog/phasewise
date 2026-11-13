+++
agent = "codex"
verify = ["npx tsc --noEmit", "npm test"]
max_retries = 2
phase_timeout_minutes = 90
verify_timeout_seconds = 900
+++

# Migrate the project to TypeScript

<!-- Shared context, sent in every phase. Describe what is true for the whole
     plan: goals, conventions, things the agent must not touch. -->

This is a Node.js project written in JavaScript (CommonJS). Migrate it to
TypeScript one directory at a time, keeping the tests green after every phase.
Do not change public behaviour.

## Phase 1: Add the TypeScript toolchain

Install `typescript` and `@types/node` as dev dependencies and add a
`tsconfig.json` with `strict: true`, `allowJs: true` and `checkJs: false`.
Add a `typecheck` script to `package.json`.

```verify
npx tsc --noEmit
```

## Phase 2: Convert src/utils

Rename the files in `src/utils` to `.ts`, add explicit types and fix every
error reported by `npx tsc --noEmit`. Keep exports unchanged.

## Phase 3: Convert src/services

Same as the previous phase for `src/services`. Replace `require` with `import`
where the module is already converted.

```verify
npm run lint
```

## Phase 4: Turn on checkJs and clean up

Set `checkJs: true`, convert whatever remains, then remove `allowJs` and any
leftover `// @ts-ignore` comments.
