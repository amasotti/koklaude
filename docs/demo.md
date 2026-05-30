# Recording the koklaude demo (S4)

A playbook for you, Toni. Goal: two artifacts for the README —

1. a **terminal GIF** showing the install/use flow (silent, embeds inline), and
2. a short **voice clip** so people can actually hear koklaude (`.m4a`, linked).

Optionally a combined screen-recording with sound (the "wow" version) — see the
end. Everything below uses tools already on the machine: `asciinema`, `agg`,
`afconvert` (macOS built-in), `gh`. No ffmpeg needed.

---

## 0. Prep (5 min)

- **Clean terminal.** Fresh shell, no clutter scrollback. Bump the font size so
  it's legible in a GIF (~16–18pt). A wide-ish window: aim for ~90 cols × ~24
  rows (we'll pin these in `agg` too).
- **Pick the lines you'll type.** Don't improvise on camera. Suggested storyboard:

  ```bash
  koklaude say "Local, offline text to speech for Claude Code."
  koklaude off          # silence
  koklaude on           # speech back
  koklaude say "Claude can speak now — fully on device."
  ```

  (Keep it short — a 15–25s cast makes a tight GIF. The init/download step is
  long and boring; skip it on camera or fast-forward via `agg --speed`.)
- **Have espeak-ng + model installed** so `say` works instantly (you do).

---

## 1. Record the terminal cast

```bash
mkdir -p docs/demo
asciinema rec docs/demo/koklaude.cast --cols 90 --rows 24
```

- This drops you into a recording shell. Type the storyboard lines slowly and
  deliberately (you can hear the audio play, but the cast only captures the
  *terminal* — that's fine, the voice clip is recorded separately in §3).
- Pause naturally; we trim idle time in conversion.
- Press **Ctrl-D** (or type `exit`) to stop. The cast is saved.

Preview it before converting:

```bash
asciinema play docs/demo/koklaude.cast
```

Don't like a take? Re-record — it's cheap. Delete and redo.

---

## 2. Convert the cast → GIF

```bash
agg \
  --cols 90 --rows 24 \
  --font-size 18 \
  --theme monokai \
  --speed 1.5 \
  --idle-time-limit 1.5 \
  docs/demo/koklaude.cast docs/demo/koklaude.gif
```

- `--speed 1.5` tightens the pace; `--idle-time-limit 1.5` caps any dead air to
  1.5s so long pauses don't bloat the GIF.
- Try a couple of `--theme` values (`asciinema`, `dracula`, `monokai`,
  `solarized-dark`) — pick what reads well.

> **Version-mismatch fallback.** asciinema 3.x writes a newer cast format than
> `agg 1.9.0` may expect. If `agg` errors on the cast version, downconvert first:
> ```bash
> asciinema convert docs/demo/koklaude.cast docs/demo/koklaude.v2.cast
> # then run agg on the .v2.cast file
> ```
> (I haven't verified the two versions interoperate on this machine — try the
> direct path first, fall back to convert if it complains.)

---

## 3. Capture the voice clip

You don't need a loopback/audio-capture tool. `koklaude say` **writes the WAV to
disk** before playing it (`playback.rs` → `$TMPDIR/koklaude-say.wav`). So:

```bash
koklaude say "Local, offline text to speech for Claude Code. No cloud, no keys."
cp "$TMPDIR/koklaude-say.wav" docs/demo/koklaude-sample.wav
```

That WAV is 24 kHz mono **IEEE-float** (the Kokoro output format — note Python's
`wave` can't parse it, but CoreAudio/`afplay` can). Convert it to a small,
broadly-playable `.m4a` with the built-in `afconvert`:

```bash
afconvert -f m4af -d aac -b 128000 \
  docs/demo/koklaude-sample.wav docs/demo/koklaude-sample.m4a
```

Sanity-check by playing the converted file:

```bash
afplay docs/demo/koklaude-sample.m4a
```

Keep the `.m4a`, drop the intermediate `.wav` if you like (note: `*.wav` is
gitignored — see §5 if you want the audio committed).

---

## 4. (Optional) Combined screen + sound recording

If you want one clip where viewers *see the terminal and hear the voice* at
once, the cast-GIF route can't carry audio. Easiest path on macOS:

- **QuickTime Player → File → New Screen Recording**, record the terminal while
  running the storyboard. QuickTime captures the **mic**, not system audio, so
  to record koklaude's own output you'd need a loopback device (e.g. BlackHole)
  routed as the input — that's extra setup.
- Simpler and usually good enough: post the silent GIF *and* the `.m4a` clip
  side by side. Skip the combined video unless you specifically want it.

---

## 5. Host & embed in the README

**The GIF** embeds inline directly — commit it and reference it:

```markdown
![koklaude in action](docs/demo/koklaude.gif)
```

**The audio** is trickier: GitHub does **not** render a player for a
committed audio file — only a download link. Two options:

- **Link it** (simplest): commit `docs/demo/koklaude-sample.m4a` and add
  `[▶ Hear koklaude](docs/demo/koklaude-sample.m4a)`. Note `.wav`/`.bin`/`.onnx`
  are gitignored, but `.m4a` is not — it'll commit fine. Keep it small (a ~5s
  clip is tens of KB).
- **Auto-embedded player** (nicer): drag the `.m4a` into a GitHub *issue* or
  *release* description in the web UI. GitHub uploads it and gives you a
  `…githubusercontent.com/…` URL that renders an inline player. Paste that URL
  into the README. (This is the only way to get a real play button on GitHub.)

Recommended README placement: a short **Demo** section right after the intro /
status banner — GIF first, audio link directly under it.

---

## Checklist

- [ ] `docs/demo/koklaude.cast` recorded, previews cleanly
- [ ] `docs/demo/koklaude.gif` generated, reads well at README width
- [ ] `docs/demo/koklaude-sample.m4a` created, plays back fine
- [ ] README gains a **Demo** section: GIF embedded + audio link/player
- [ ] (optional) audio uploaded via GH UI for an inline player URL

When the artifacts exist, ping me — I'll wire the **Demo** section into the
README and we can close Phase 6.
