# Changelog

All notable changes to this project will be documented in this file (created with git cliff).

## [0.0.3] - 2026-05-31

[Compare with last version](https://github.com/amasotti/koklaude/compare/6f21201b3f753bc6ef40fbf1b11e9eb0085744f8..a006422bf59f62c6620473af87c1b7860b201a08)
### 🐛 Bug Fixes


- Uninstall command ([2b32af8](https://github.com/amasotti/koklaude/commit/2b32af8e0faf106627538cc0c3f4677b7d3e91b1))

- Transcript read race (flaky hook in terminal) ([a006422](https://github.com/amasotti/koklaude/commit/a006422bf59f62c6620473af87c1b7860b201a08))

### ⚙️ Miscellaneous Tasks


- Update cargo.lock ([461f6fc](https://github.com/amasotti/koklaude/commit/461f6fc8f4b1d0e0d3112d15babc404318c7f6d3))

## [0.0.2] - 2026-05-30

[Compare with last version](https://github.com/amasotti/koklaude/compare/1a3b75bd5013a5ef8280431692a2266eacc75e29..6f21201b3f753bc6ef40fbf1b11e9eb0085744f8)
### ⚙️ Miscellaneous Tasks


- Release pipelines ([daa3966](https://github.com/amasotti/koklaude/commit/daa396641c39955e91e4ef3fe8392fd47c6641ca))

- Release v0.0.2 ([6f21201](https://github.com/amasotti/koklaude/commit/6f21201b3f753bc6ef40fbf1b11e9eb0085744f8))

## [0.0.1] - 2026-05-30

### 🚀 Features


- Init project ([dd53587](https://github.com/amasotti/koklaude/commit/dd53587f0f64d41b0d1ed854392f98f243c17a3e))

- Phase 0 - scaffold structure ([4a04cd9](https://github.com/amasotti/koklaude/commit/4a04cd96f541ff7eca36df7dccb51bd3cd95ea50))

- Spike to see if local kokoro works ([14e881c](https://github.com/amasotti/koklaude/commit/14e881cf969e50107d463dc43ea792109cbc7124))

- More complete spike ([91f3e5f](https://github.com/amasotti/koklaude/commit/91f3e5fb9cceaf8834f76d41ff6d250f211b55a0))

- Skeleton for the hanasu engine ([8482c09](https://github.com/amasotti/koklaude/commit/8482c095bf80692a155b6895b9f15d89221b1010))

- Voice module for hanasu ([683fd4e](https://github.com/amasotti/koklaude/commit/683fd4edeef99cc17ce09a89d4d88fcff24b42eb))

- Call to external espeak-ng ([da5525d](https://github.com/amasotti/koklaude/commit/da5525d5a2276cbd2ba16574ab1fb10192ef2bf7))

- Hanasu tokenizer ([5fa91d7](https://github.com/amasotti/koklaude/commit/5fa91d702d401823c001e0e47a4a1a711ea5dd27))

- Put together all parts of the engine ([38ad5e0](https://github.com/amasotti/koklaude/commit/38ad5e09aa66281882fff52a0252537336f5830f))

- Config frontend koklaude ([0ad0c56](https://github.com/amasotti/koklaude/commit/0ad0c56f0b5a550eefc9a8c42407430de92edd9b))

- Playback configured ([cc9cb06](https://github.com/amasotti/koklaude/commit/cc9cb060df6b35c35feab36c4e448faf0775a722))

- Add configuration ([ab14eb7](https://github.com/amasotti/koklaude/commit/ab14eb726e295babe334c470339d2fc49483b9cc))

- Clean markdown with pulldown-cmark ([49a1a31](https://github.com/amasotti/koklaude/commit/49a1a313c6cd46c8d31f85d8b832479d07d1bf9f))

- Jsonl transcript ([f24f872](https://github.com/amasotti/koklaude/commit/f24f8723caf9f40b89f1faefd34bd24023ffadaf))

- Socket path and wire protocol ([838dfee](https://github.com/amasotti/koklaude/commit/838dfeed65ea2451c97168e31897b030eedfc486))

- Daemon idle shutdown and bail on error ([eeb43fd](https://github.com/amasotti/koklaude/commit/eeb43fd27c4a1a25b65c1e5b36a171901d5048e2))

- Poll-connect with backoff until ready ([e5f2c6e](https://github.com/amasotti/koklaude/commit/e5f2c6e570e365ea1f3117fde3dceffe6df35b32))

- Pure reply_to_speak ([68ded23](https://github.com/amasotti/koklaude/commit/68ded238bff404c1e11b581f721f1fcc3e931fe1))

- Setup foundation ([da4936c](https://github.com/amasotti/koklaude/commit/da4936ccae6746d20694ed545d920eb13caa315e))

- Check prerequisites ([755405d](https://github.com/amasotti/koklaude/commit/755405dafb81a8b902c8ce608aab14d90e152f25))

- Download model and voices ([99b2c40](https://github.com/amasotti/koklaude/commit/99b2c4013dd5dedebdcb74ff12680f208b6b664c))

- Harden dangerous actions ([30f5eea](https://github.com/amasotti/koklaude/commit/30f5eea504212da357e28ada4de829ff7f9d177e))

- Harden dangerous actions ([1a3b75b](https://github.com/amasotti/koklaude/commit/1a3b75bd5013a5ef8280431692a2266eacc75e29))

### 🐛 Bug Fixes


- Espeak error on dash starting texts ([fdd1692](https://github.com/amasotti/koklaude/commit/fdd169202507c338b094a507afceb2e1598d8b40))

- Espeak version flaky tests ([78d10d0](https://github.com/amasotti/koklaude/commit/78d10d0297cf9f85d4c5f052f3c6dad8aac10c94))

- Code review findings ([05d8529](https://github.com/amasotti/koklaude/commit/05d85296d80fe969cb353f604a96d81d29bde525))

### 🚜 Refactor


- Use assistant agnostic folder ([d070d98](https://github.com/amasotti/koklaude/commit/d070d98ca6df4a59c54716ba49132c826c5d4f43))

### ⚙️ Miscellaneous Tasks


- *(ci)* Setup github actions and tooling ([ca75652](https://github.com/amasotti/koklaude/commit/ca75652ced9b638f7393adf4a7a46dca48a10b2b))

- Fmt ([cfcb631](https://github.com/amasotti/koklaude/commit/cfcb631c7c0d37b031692c82d07249824a3add39))

- Add cargo-deny ([1742c0d](https://github.com/amasotti/koklaude/commit/1742c0dea5eab4d05049e71f798cc5138ce1b390))

- Complete koklaude setup ([c881fa4](https://github.com/amasotti/koklaude/commit/c881fa49d85ba5932e27c4d80075b45bff0a941d))

<!-- generated by git-cliff -->
