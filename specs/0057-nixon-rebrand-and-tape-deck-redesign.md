# 0057 — "Nixon": rebrand + reel-to-reel design language

- **Status:** Proposal — decisions locked 2026-09-13 (see "Decisions locked"); mockup at https://claude.ai/code/artifact/a08f1448-3b8a-412d-af03-3edb91ab694a; awaiting go for implementation plan
- **Owner agent(s):** spec-architect (this doc); implementation later by frontend-engineer
  (+ rust-core-engineer for the rename surfaces, icon/tray, and the `recording-level` wiring)
- **Roadmap phase:** Phase 5 (Polish) — runs alongside feature work, not instead of it

> This is a design deliverable. It changes no code. It proposes a name, a complete token
> replacement for `frontend/src/app/globals.css`, six signature components, and a phased
> rollout. It respects the IA and flow decisions locked in `specs/0007` — every change below
> is skin, geometry, and state legibility, not navigation.

---


## Decisions locked (2026-09-13, owner review of round-2 mockup)

These override anything to the contrary in §2–§8 below.

1. **Theme: light ("Faceplate") is the default; follow the system dark-mode setting; add a
   manual override in Settings (Faceplate / Deck / System, default System-with-light).**
   The "dark is primary" argument in §2 is superseded. Both palettes ship at parity.
2. **Palette is warmed.** The walnut cheek stays, so the greys move off pure neutral toward
   the walnut: Faceplate ground `#E6E1D8` (putty aluminum), panel `#DED8CE`, card `#F3EFE7`,
   paper `#F7F2E7`; Deck ground `#1A1816`, card `#242120`, panel `#2C2826`, paper `#26221F`.
   Convert to HSL in `globals.css` at implementation; the mockup's hex values are the source.
3. **A little skeuomorphism is fine.** Panels and keys carry a subtle top-to-bottom sheen
   (`--sheen`, ~4% light-to-dark), cards get a real 1px shadow, the rail casts a soft
   upward shadow. The "no blur, bevel only" rule in §2 relaxes to *restraint*, not
   prohibition. Rendered screws, wood grain images, and click sounds remain out.
4. **Two metaphors, not one.** The *deck* (Rams/Braun/Sony: engraved labels, lamps, keys,
   meters) owns every control. The *typewriter on paper* owns the content the deck produced:
   the transcript body and the reel label are set in **Courier Prime** (Google Fonts) on a
   `--paper` ground with `--paper-ink` text. Summary and notes stay IBM Plex Sans — they are
   documents you author, not tape. Font roster becomes Archivo / Archivo Narrow / IBM Plex
   Sans / IBM Plex Mono / Courier Prime.
5. **Metaphor stays off the content nouns** (§1 stands). Meetings are meetings. The only
   metaphor on content is the `REEL nnnn` label card on Meeting details.
6. **VU: needle on the live screen, 10-segment ladder everywhere compact.** The ladder
   *replaces* the spectrometer (`useRecordingWaveform`, `recording-spectrum`) in the rail,
   device picker and list rows — a single level meter, not bands. The spectrum strip is
   retired from the UI (the event can stay for now).
7. **One persistent bottom rail on every screen** (`TransportBar` `docked` only; the `header`
   variant is dropped). It owns: recording status (reels, title, counter, ladder), the
   REC/HOLD/STOP keys, and the **one global QUEUE**. The rail is the single place recording
   controls live — they no longer move between Today, Record and Meeting details.
8. **One global queue.** `LlmActivityRow` (sidebar lamp strip in §3.6) is **deleted**; the
   deferred-transcription backlog and queued LLM work (summaries, task extraction, prep) merge
   into one ordered list in the rail's right zone with a lamp (amber = running, red = retry)
   and a popover showing every item with its stage. `ProcessMeetingsButton` on Today stays as
   the "run the queue now" key.
9. **Live Record header simplifies:** title + identity line on the left; the two needle VU
   meters + PEAK / MIC GATE / DROPOUT lamps on the right. No counter and no reels in the
   header — both live only in the rail.
10. **Sidebar collapses to a 64px icon rail** (icons only, labels hidden, active index bar
    kept, wordmark replaced by the ⊙—⊙ mark). Widths remain 64/256 so `MainContent`'s
    offset logic is unchanged.
11. **Bundle identifier `ai.vinyl.app` stays for now.** Expect to change it before opening
    the app to other users; when that happens `data_migration.rs` runs a second hop. Meanwhile
    Phase A must scrub every *visible* Vinyl/Meetily reference (UI strings, docs, scripts,
    package names, release tag prefix, `~/Movies/meetily-recordings/` display name) — the
    owner does not want confusing legacy names hanging around for future users.
12. **Rollout: one branch**, not the `specs/0014` wave pattern. Phases A–D in §7 are the
    commit order inside that branch, each gated on `pnpm lint && pnpm test && tsc` and an
    owner smoke pass before the branch merges.

## Context / Problem

The app is a local-first meeting recorder whose entire value proposition is *"the tape stays
in the room."* Its current skin — **Warm Editorial** (`globals.css:80-152`: paper `40 37% 97%`,
clay `--brand: 16 61% 47%`, Source Sans 3 + Newsreader) — is a competent Granola-adjacent
document look. It is also indistinguishable from thirty other productivity apps, and it says
nothing about what the product *is*. The product is a **recorder**. The interface is a
**control surface** that happens to also display documents.

The owner wants it rebranded **Nixon**, after the White House taping system, and dressed in
the visual language of 1970-era reel-to-reel decks: Braun TG 1000, Sony TC-series, Revox A77,
Nagra IV-S. That is a strong, specific, defensible direction — provided it stays Rams-ian
(*"as little design as possible"*) and never becomes a rendered-screws pastiche.

There is also a concrete craft debt this redesign should absorb, surfaced while auditing the
recording chrome:

- **"Paused" has three different colors**: `bg-secondary` + muted text
  (`components/Record/RecordingHeader.tsx:244`), `bg-orange-500`
  (`components/RecordingControls.tsx:471`, `components/TranscriptEmptyState.tsx:38`,
  `components/RecordingStatusBar.tsx:41`), and "the dot stops blinking"
  (`components/GlobalRecordingBar.tsx:129`).
- **"Live" has two colors**: `bg-record` (header pill, both waveforms) vs `bg-brand`
  (`components/TranscriptEmptyState.tsx:41`, `components/VirtualizedTranscriptView.tsx:823`).
- **285 raw Tailwind palette classes** across `frontend/src` bypass the token set entirely —
  including the amber backpressure banner (`app/_components/TranscriptPanel.tsx:156`), the red
  device-error alert (`components/RecordingControls.tsx:494`), and the whole speaker palette
  (`lib/speaker-colors.ts:12-21`). A theme swap that does not fix these produces a broken
  dark mode.
- **Two bottom-center pills occupy identical coordinates** and can co-occur:
  `GlobalRecordingBar` (`app/layout.tsx:307`) and `DeferredBacklogIndicator`
  (`app/layout.tsx:310`), both `fixed bottom-6 … z-50 … rounded-2xl bg-foreground shadow-xl`.
- **`recording-level` is emitted and nobody listens.** `audio/pipeline.rs:1079-1085` emits
  `{ rms, peak }` every 80 ms (~12.5 Hz, `pipeline.rs:62`). Grep finds zero frontend
  consumers. A real VU meter is already fed by the backend; it just has no face.
- **The mute-gate (`specs/0049`) has no live UI at all.** When the Zoom gate silences the mic
  mid-recording, the header still shows a pulsing "Recording" pill and a moving waveform.

## Goals

- A name, a voice, and a palette that are unmistakably this product's.
- A **complete** token replacement in `globals.css` with light *and* dark parity, dark primary.
- Six signature components that carry the identity and simultaneously repair the state-color
  inconsistencies above.
- **Zero added clicks.** Every `specs/0007` flow (one-key start ⌘⇧R, persist-at-start,
  back-to-back use, ⌘K) survives untouched.

## Non-goals

- Changing the IA locked in `specs/0007` / rolled out in `specs/0014`. Home is still the
  dashboard, `/record` is still the live screen, meeting details still has its tabs.
- Skeuomorphic rendering: no photographic textures, no bitmap knobs, no audio feedback.
- A new bundle identifier (argued in §7).
- Any backend change beyond one tray icon and the optional mute-gate event.

---

## 1. Brand concept

### What "Nixon" means here

The joke is a **one-liner, delivered once, deadpan, and then dropped.** Nixon wired the Oval
Office to record everything he said, and the tapes destroyed him because *somebody else got
them*. This product records everything you hear and the tapes never leave the machine. That
is the entire gag and the entire positioning, and they are the same sentence: **the problem
was never the taping, it was the subpoena.**

That is why the name works for a privacy product rather than against it. The reference is not
"be paranoid like Nixon"; it is "keep your own record, on your own hardware, answerable to
nobody." It is also self-aware — a meeting recorder that names itself after the most famous
meeting recorder in history is not pretending to be neutral infrastructure.

**Tone of voice:** dry, declarative, mechanical. Short sentences. Verbs from the control
surface (*armed, running, held, stopped, on the reel*). Never chirpy, never exclamation
marks, never "Oops!". Where the current copy says *"Welcome to Vinyl!"*
(`components/TranscriptEmptyState.tsx:51`), Nixon says *"Deck ready. Nothing on the reel yet."*
The humor is in the restraint, not in puns.

### Taglines

1. **"Records everything. Tells no one."** ← recommended primary
2. "Your own tapes, on your own machine."
3. "Everything on the record. Nothing off the Mac."
4. "No gaps." (a dry nod to the 18½-minute erasure, read as a *reliability* claim)
5. "The tape never leaves the room."

(1) is the recommendation: it states the product in five words, lands the joke without naming
it, and doubles as the App Store subtitle and the About line. (4) is the best deep cut but
only works for people who get it; keep it for the release-notes footer or the empty state.

### Sub-feature naming — mostly don't

The temptation is to rename everything: Meetings → *Reels*, the backlog → *The Queue*,
Ask AI → *The Archivist*. That is where this kind of rebrand goes to die. Search, scanning,
and onboarding all get worse when the nouns are cute.

**The rule: metaphor on the controls, plain language on the content.**

| Keep plain (content) | Adopt the metaphor (controls) |
|---|---|
| Meetings, Notes, Summary, Transcript, People, Action items, Settings | **REC / HOLD / STOP** transport keys |
| "Recording", "Paused", "Processing" as status *words* | **LEVEL** (the VU meter), **COUNTER** (elapsed) |
| Meeting titles, dates, attendees | **CH 1 / CH 2 / CH 3** for the speaker legend |
| Ask AI | **LAMP** states: `READY` / `RUN` / `FAIL` for background LLM work |
| | **QUEUE** for the deferred-processing backlog |

One indulgence is allowed, and only one: the **About panel and the tray tooltip** may carry
the wink — *"Nixon · recording since 3 Jun 2026 · 412 reels."* Everything else stays boring.

`PAUSE` becomes **`HOLD`** on the transport key only (decks label it *pause*, but `HOLD`
reads better in caps at 10px and disambiguates from playback). The accessible name stays
"Pause recording" so the existing `aria-label`s and tests
(`components/GlobalRecordingBar.tsx:172`) don't churn.

**Risk, one line:** "Nixon" is a live commercial mark in watches/eyewear/accessories and a
real person's surname — clearance is a lawyer question in the software class, and the product
must never use Richard Nixon's likeness, voice, signature, or the presidential seal (the seal
is separately protected under 18 U.S.C. §713). Keep the reference verbal and self-deprecating;
never illustrate it.

---

## 2. Design language

### The object we are imitating

Not a *picture* of a tape deck — the **logic** of one. A 1970 Revox is legible in the dark
from four feet away because it obeys four rules, and those four rules are the whole design
language:

1. **One function, one control, one label.** Nothing is modal, nothing is hidden behind a
   hover.
2. **State is emitted, not described.** A lamp is on or off. A needle sits where the signal
   is. You never read a sentence to learn whether the machine is running.
3. **Hierarchy is material, not decoration.** The transport keys are physically bigger and
   sit on a different plane than the trim pots. There are no "primary/secondary" *colors* —
   there is a *size* and a *plane*.
4. **The panel is quiet so the signal is loud.** Charcoal and cream everywhere; the only
   saturated colors in the entire machine are the lamps.

### Materials (flat color + one gradient + almost no noise)

| Material | How to build it | Where |
|---|---|---|
| **Anodized charcoal chassis** | flat `--background`. Nothing else. | app body, page ground |
| **Brushed aluminum panel** | flat `--panel` + a 1px top bevel `inset 0 1px 0 hsl(var(--bevel-hi))` and 1px bottom `inset 0 -1px 0 hsl(var(--bevel-lo))` + a **1px-period** repeating gradient at 1.2% alpha: `repeating-linear-gradient(90deg, transparent 0 2px, hsl(0 0% 100%/.012) 2px 3px)` | sidebar, record header rail, transport bar |
| **Engraved label** | `--engrave` fill + `text-shadow: 0 1px 0 hsl(0 0% 100%/.06)` in dark (light catches the lower lip of a groove), `0 -1px 0 hsl(0 0% 100%/.7)` in light | all-caps panel labels, the wordmark |
| **Indicator lamp** | flat `--brand` / `--record` / `--success` fill + a `box-shadow: 0 0 0 1px <color>/.25, 0 0 8px -1px <color>/.55` halo. **No blur filters.** | REC, LEVEL over-zone, LAMP row |
| **Walnut cheek** | flat `hsl(22 30% 22%)`, a **6px** vertical strip at the window's left edge, behind the sidebar. No texture, no grain image. | app chrome only; opt-out in Settings |

Global grain is optional and must be almost invisible: one inline SVG `feTurbulence`
(`baseFrequency=.9`, `numOctaves=3`) at `opacity: .022; mix-blend-mode: overlay`, fixed to the
viewport, `pointer-events:none`, and disabled under `prefers-reduced-transparency`. If it is
visible as texture, it is wrong — turn it off.

### Typography

Three families, all Google Fonts, all loadable through the existing `next/font/google` setup
in `app/layout.tsx:43-54`.

| Role | Face | Usage |
|---|---|---|
| **Panel / UI / labels / counters** | **Archivo** (400/500/600/700) | nav, buttons, transport labels, headings, all numerals via `font-variant-numeric: tabular-nums` |
| **Meter scales & tick numerals** | **Archivo Narrow** (400 only) | the VU scale (`-20 -10 -7 -5 -3 0 +3`), channel-strip micro-labels, table column heads |
| **Long-form reading** | **IBM Plex Sans** (400/500/600) | transcript body, summary document, notes editor |
| **Timecodes / machine strings** | **IBM Plex Mono** (400/500) | transcript gutter timestamps, model names, file paths |

**Why not Helvetica Neue**, which is the historically exact answer and already on every Mac:
its macOS cut has no tabular figures, hints poorly below 13px, and its tight apertures make a
dense transcript tiring. Archivo is an Akzidenz/DIN-descended grotesk with a genuinely good
tabular set and a Narrow sibling — it *reads* as silkscreen at 10px caps and as a usable UI
face at 13px, which Helvetica does not. Keep `"Helvetica Neue", -apple-system` as the
fallback stack so a cold start still looks right.

**The engraved-label spec** (used everywhere a 1970 panel would have silkscreen):
Archivo 600, `10px`, `text-transform: uppercase`, `letter-spacing: .14em`, color `--engrave`.
This replaces the current `.u-section-label` (`globals.css` `@layer components`), which stays
as the class name — only its body changes, so nothing has to be renamed at call sites.

Body copy drops from the current Source Sans 3 to IBM Plex Sans at the same 14px
(`globals.css` `.note-editor`, `.summary-doc`), so line counts and the BlockNote layout do not
move. Headings lose the Newsreader serif entirely — a serif in a machine panel is the single
fastest way to make this look like a costume. `h1, h2, .font-display` becomes Archivo 600 with
`letter-spacing: -.011em`.

### Color system — full replacement for `globals.css`

**Dark is primary.** The argument: (a) this app is a *control surface* first — the identity
moment is a live recording, and amber and red lamps simply do not exist as signals on a
`40 37% 97%` paper ground; (b) it runs all day beside a Zoom window, frequently in a dim room;
(c) every reference object — Nagra IV-S, the black TC-800, the charcoal A77 face — is dark
with cream legends, and the aluminum-faced ones are *silver*, not white; (d) Granola is light,
so dark is also the differentiator. The counter-argument is real — long-form transcript and
summary reading is worse on black — and it is answered structurally, not by giving up:
`--card` sits a full 4 points of lightness above `--background`, the document column renders
on `--card`, and `--foreground` is held at **88%** lightness rather than white to kill halation.

Light mode ("Faceplate") is not a second-class citizen — it is the brushed-aluminum Braun, and
it must ship at parity.

```css
@layer base {
  /* ---- DECK (dark) — default ------------------------------------------- */
  .dark {
    --background: 195 6% 9%;        /* anodized chassis            #15181A */
    --foreground: 42 22% 88%;       /* cream silkscreen            #E9E2D6 */
    --card: 200 5% 13%;             /* panel face                  #1F2224 */
    --card-foreground: 42 22% 88%;
    --popover: 200 6% 16%;
    --popover-foreground: 42 22% 90%;
    --primary: 42 22% 88%;          /* cream key, dark legend */
    --primary-foreground: 195 8% 10%;
    --secondary: 200 5% 17%;
    --secondary-foreground: 42 16% 80%;
    --muted: 200 5% 15%;
    --muted-foreground: 40 7% 57%;  /* 5.3:1 on background */
    --accent: 200 5% 21%;
    --accent-foreground: 42 22% 90%;
    --destructive: 4 64% 47%;
    --destructive-foreground: 42 26% 94%;
    --border: 200 6% 22%;
    --input: 200 6% 19%;
    --ring: 38 90% 56%;
    --radius: 0.25rem;

    --brand: 38 90% 56%;            /* AMBER LAMP — armed/active    #F0A72A */
    --brand-foreground: 195 10% 8%;
    --record: 4 76% 53%;            /* REC LAMP                     #E24A3D */
    --record-foreground: 40 30% 96%;
    --success: 128 34% 46%;         /* READY LAMP                   #4F9E58 */
    --success-foreground: 195 8% 10%;

    --chart-1: 38 90% 56%;          /* CH1 — You (amber) */
    --chart-2: 186 44% 52%;         /* CH2 — cyan lamp */
    --chart-3: 42 20% 78%;          /* CH3 — cream */
    --chart-4: 22 62% 52%;          /* CH4 — ochre */
    --chart-5: 268 28% 66%;         /* CH5 — violet-grey */

    /* new tokens (add to tailwind.config.js colors) */
    --panel: 200 5% 17%;            /* brushed rail, distinct from --card */
    --engrave: 40 6% 62%;           /* silkscreen label ink */
    --bevel-hi: 0 0% 100% / .055;
    --bevel-lo: 0 0% 0% / .45;
    --meter-over: 4 76% 53%;        /* the red zone above 0 VU */
  }

  /* ---- FACEPLATE (light) ----------------------------------------------- */
  :root {
    --background: 40 10% 90%;       /* brushed aluminum            #E8E5E0 */
    --foreground: 200 10% 13%;
    --card: 40 16% 95%;
    --card-foreground: 200 10% 13%;
    --popover: 40 20% 97%;
    --popover-foreground: 200 10% 13%;
    --primary: 200 10% 15%;
    --primary-foreground: 40 20% 96%;
    --secondary: 40 9% 85%;
    --secondary-foreground: 200 10% 18%;
    --muted: 40 9% 86%;
    --muted-foreground: 200 6% 40%;  /* 4.6:1 on background */
    --accent: 40 10% 81%;
    --accent-foreground: 200 10% 15%;
    --destructive: 4 68% 42%;
    --destructive-foreground: 40 24% 96%;
    --border: 40 8% 74%;
    --input: 40 8% 78%;
    --ring: 28 80% 40%;
    --radius: 0.25rem;

    --brand: 28 78% 38%;
    --brand-foreground: 40 24% 97%;
    --record: 4 70% 44%;
    --record-foreground: 40 28% 97%;
    --success: 132 40% 28%;
    --success-foreground: 40 24% 97%;

    --chart-1: 28 78% 38%;
    --chart-2: 190 52% 30%;
    --chart-3: 200 10% 28%;
    --chart-4: 22 62% 36%;
    --chart-5: 268 30% 45%;

    --panel: 40 12% 87%;
    --engrave: 200 8% 38%;
    --bevel-hi: 0 0% 100% / .8;
    --bevel-lo: 0 0% 0% / .13;
    --meter-over: 4 70% 44%;
  }
}
```

**One rule that must be enforced**: `--record` red is a *lamp*, not a text color. At
`4 76% 53%` on `195 6% 9%` it is 4.6:1 — legal for ≥14px but tight, and it is currently used
as 11px text (`RecordingHeader.tsx:248`). In the new system, "REC" is always **cream text on a
lit red key**, never red text on the deck.

### Radius, elevation, borders, icons, motion

- **`--radius: 0.25rem` (4px).** Transport keys 2px. Meter bezel 2px. Panels 3px. Popovers
  4px. **Nothing is a pill.** Killing `rounded-full` / `rounded-2xl` from the recording chrome
  (`GlobalRecordingBar.tsx:117`, `RecordingControls.tsx:342,393,415`, the status chips at
  `RecordingHeader.tsx:244-248`) is the single highest-impact geometric change in this spec —
  a machined panel has no capsules on it.
- **Elevation is a bevel, not a blur.** Replace `shadow-sm`/`shadow-lg`/`shadow-xl` on panels
  with the 1px `--bevel-hi` / `--bevel-lo` inset pair. Only things that are genuinely *above*
  the deck — Dialog, Popover, DropdownMenu, the ⌘K palette — keep a real shadow:
  `0 24px 48px -16px hsl(0 0% 0% / .65)`.
- **Borders do the work.** 1px `--border` on every panel edge; groups of controls are
  separated by a 1px rule, not by whitespace.
- **Icons:** keep lucide, but set `strokeWidth={1.5}` and `strokeLinecap="square"`
  `strokeLinejoin="miter"` globally via a thin wrapper — lucide's round caps are the most
  "2020 SaaS" thing in the app and squaring them costs one file. The five **transport glyphs**
  (● ▶ ❙❙ ■ ⏏) are hand-drawn SVG at exact pixel sizes, solid fill, no stroke — they must be
  optically identical to a deck's.
- **Motion is mechanical.**
  - *Key press:* 60 ms, `cubic-bezier(.2,0,0,1)`, 1px downward translate + bevel inversion.
    No scale, no bounce.
  - *Lamp:* 120 ms attack (incandescent filament ramp), 400 ms decay. Never instant, never
    fading over a second.
  - *Needle:* true VU ballistics — 300 ms to 99% of a step input, symmetric return, ~1.5%
    overshoot. Implemented as a critically-damped-ish second-order integrator on `rAF`, not a
    CSS transition (see §3).
  - *Counter digits:* 110 ms vertical roll with a 4% overshoot on the settling digit.
  - *Reels:* constant angular velocity, supply reel 0.85 rev/s.
  - *Panel/route transitions:* 140 ms ease-out opacity only. Remove the existing
    `motion.div` y:20 rise on `/record` (`app/record/page.tsx:134`) and the `animate-vibrate`
    / `fade-in-up` keyframes in `globals.css` — springy entrances are the opposite of this.

---

## 3. Signature components

Six elements carry the identity. Each is a **new file** (the ratchet in
`scripts/check-file-size.sh` forbids growing allowlisted files, and
`components/RecordingControls.tsx` is already 524 lines), and each replaces something that
exists today.

### 3.1 The Transport — `components/Transport/TransportBar.tsx`

Replaces **three** surfaces that today do the same job three different ways: the floating pill
(`RecordingControls.tsx:339-525` + the double-wrapping at `app/record/page.tsx:195`), the
header button pair (`RecordingControls.tsx:267-337`), and `GlobalRecordingBar.tsx:117`. One
component, one `variant` prop (`header | docked`), one state vocabulary.

Illuminated keys, not buttons: a 44×34 key with 2px radius, a 1px bevel, an engraved 9px
caps legend, and a **lamp inset** — a 3px cream bar across the key's top edge that lights in
the key's own color when that function is engaged. `REC` lamp = `--record`. `HOLD` lamp =
`--brand` amber. `STOP` is never lit (it is the absence of state).

```
 ┌──────────────────────────────────────────────────────────────────────────────┐
 │  ▓▓▓▓  ← lamp bar (lit red)                                                   │
 │ ┌──────┐┌──────┐┌──────┐  ╎  ⊙⊙  00:04:12  ╎ ▁▃▅▇█▅▃▁▁  ─20 ─7 0 +3         │
 │ │  ●   ││  ❙❙  ││  ■   │  ╎ reels  COUNTER  ╎     LEVEL                       │
 │ │ REC  ││ HOLD ││ STOP │  ╎                 ╎                                 │
 │ └──────┘└──────┘└──────┘  ╎                 ╎                                 │
 └──────────────────────────────────────────────────────────────────────────────┘
   lit      dim     dim        1px rules separate functional groups
```

State table — this is the fix for "paused has three colors":

| State | REC key | HOLD key | Reels | Counter | Level |
|---|---|---|---|---|---|
| idle | dim, pressable | dim, disabled | still | `00:00:00` dim | flat at rest |
| recording | **lit red**, latched down | dim | turning | counting, cream | live |
| paused (`HOLD`) | lit red, **dimmed to 55%** | **lit amber**, latched | still | counting **frozen**, amber | frozen at last value |
| mic-gated (Zoom mute, `specs/0049`) | lit red | dim | turning | counting | **`MIC` lamp lit amber**, level shows system-only |
| finalizing | dim | dim | slowing to still | frozen | flat, `SAVING` engraved |

The mic-gate row is new UI that does not exist today — the gate fires silently
(`ZoomMuteGateToggle.tsx` is settings-only, no listener anywhere in `src/`). A single amber
`MIC` lamp beside the meter is the whole fix and it needs one Rust event.

`docked` variant replaces the floating pill and sits **flush to the bottom edge of the window**
as a full-width 44px rail — not a floating capsule. That also resolves the
`GlobalRecordingBar` / `DeferredBacklogIndicator` collision (`layout.tsx:307` vs `:310`): the
rail has a left zone (transport) and a right zone (queue/lamp), so they stack instead of
overlapping. Existing `aria-label`s and the `requestFullRecordingStop` routing
(`GlobalRecordingBar.tsx:28-35`) are preserved verbatim.

### 3.2 The VU meter — `components/Transport/VuMeter.tsx`

Replaces `components/AudioLevelMeter.tsx` (whose tick markers at `:89-96` are positioned
wrong anyway) and unifies the three duplicated bar-graph implementations
(`RecordingHeader.tsx:46-78` 28 bars / `GlobalRecordingBar.tsx:208-238` 14 bars /
`record/page.tsx:94-98` 3 bars).

**Needle, not bar graph — and this is the one place to spend the effort.** The argument is
technical, not nostalgic: the backend emits at 12.5 Hz (`audio/pipeline.rs:62`). A segmented
LED bar at 12.5 Hz reads as a stutter; a needle with proper VU ballistics *integrates* the
signal and 12.5 Hz is more than enough to drive it, because a VU meter's entire job is to be
slow. Standard ballistics: **300 ms to 99% of a step input, symmetric fall, ≤1.5% overshoot**.
Implement as a second-order integrator stepped on `requestAnimationFrame` from the latest
`recording-level` `{ rms, peak }` — **an event that already exists and currently has zero
consumers** (`pipeline.rs:1079-1085`). Scale is `20·log10(rms)` mapped to the classic VU
face: `-20 -10 -7 -5 -3 0 +3`, with 0 VU at ~72% of arc travel.

```
    ┌───────────────────────────────────────────┐
    │   ‑20   ‑10  ‑7  ‑5  ‑3   0  +3           │   ← Archivo Narrow 8px, --engrave
    │    ·  ·  ·  ·  ·  ·  · ╎▏▎▍  ▍▎▏          │   ← ticks; red zone right of 0
    │            ╲                              │
    │             ╲___                          │   ← needle: 1.5px, --foreground
    │                                           │
    │  ◉ CH1 MIC        ◉ CH2 SYS        ▪ PEAK │   ← two meters, one per channel
    └───────────────────────────────────────────┘
```

Two meters side by side (mic / system) is the honest thing — the app genuinely has two
channels, `pipeline.rs` mixes them, and showing both makes the Zoom mute-gate visible without
any new copy. A separate **PEAK** lamp latches red for 800 ms whenever `peak > 0.98`.

In the compact contexts (docked transport rail, device selection) the needle degrades to a
**single horizontal ladder of 14 segments** with the same ballistics and the same over-0 red
zone — same math, less arc. The spectrometer (`specs/0025`, `useRecordingWaveform`) is kept
but demoted from "the recording indicator" to a decorative strip; it stays available under a
`showSpectrum` prop for the record screen where there is room.

### 3.3 The tape counter — `components/Transport/TapeCounter.tsx`

**Mechanical odometer digits, not 7-segment and not Nixie.** Seven-segment is 1976-and-later
LED calculator vocabulary, renders badly below 20px, and reads as "cheap Casio"; Nixie is a
1960s instrument tube that never appeared on a consumer deck and is now internet kitsch. Every
reference machine — A77, TC-800, Nagra — has a **mechanical roller counter**, and that is also
the easiest thing to render crisply at any size: tabular Archivo digits in individual 
recessed wells, each digit rolling vertically 110 ms when it changes.

```
 ┌──┐┌──┐┌──┐ ┌──┐┌──┐ ┌──┐┌──┐
 │0 ││0 ││: │ │0 ││4 ││: ││1 │2    ELAPSED
 └──┘└──┘└──┘ └──┘└──┘ └──┘└──┘
   Archivo 600, tabular-nums, tracking -.02em, each well 1px inset bevel
```

Format `h:mm:ss` always (leading zeros shown — a counter does not hide its digits), replacing
`formatElapsed`'s conditional hour (`GlobalRecordingBar.tsx:241-254`,
`RecordingHeader.tsx:31-39`). Sizes: 28px in the record header, 15px in the docked rail, 13px
in list rows for durations. Same component drives the transcript gutter timestamps at 11px,
which unifies a fourth duplicated formatter.

### 3.4 The reels — `components/Transport/Reels.tsx`

The recording indicator. Two 18px hubs, three spoke slots each, joined by a 1px tape path.
When recording, both turn at **0.85 rev/s**, the supply hub's slot-ring shrinking and the
take-up's growing over the session (a real, honest progress signal: how much of the meeting
has elapsed). Paused: they stop dead — no fade, no slow-down, the way a solenoid brake stops
a reel. Finalizing: 600 ms spin-down.

This replaces the blinking dot in three places (`GlobalRecordingBar.tsx:127`,
`RecordingHeader.tsx:249`, `Today/TimelineBlock.tsx` `animate-ping`) with a single
non-blinking indicator. **Blinking is the worst possible affordance for a thing that runs for
90 minutes** — it is an alarm pattern, and the current app blinks at you for the entire
meeting. Rotation is strictly `prefers-reduced-motion`-gated: when reduced, the reels render
statically with the amber lamp lit and the counter carries the liveness.

### 3.5 The channel strip — `components/MeetingDetails/ChannelStrip.tsx`

Replaces the chip cloud in `components/MeetingDetails/SpeakerLegend.tsx:171` (a
`flex-wrap … max-h-32 overflow-y-auto` bag of `rounded-full` pills). A multitrack deck
labels its inputs in a fixed-width column, and so should this:

```
 ┌─────────────────────────────────────────────────────────┐
 │ CH  SPEAKER            TIME      LEVEL                   │
 ├─────────────────────────────────────────────────────────┤
 │ 1 ▌ You                24:10  ███████████████░░░░░  58% │
 │ 2 ▌ Sarah Chen         11:32  ███████░░░░░░░░░░░░░  27% │
 │ 3 ▌ Speaker 3      ✎   04:18  ███░░░░░░░░░░░░░░░░░  10% │
 │ 4 ▌ Speaker 4      ✎   02:05  █░░░░░░░░░░░░░░░░░░░   5% │
 └─────────────────────────────────────────────────────────┘
   ▌ = 3px channel color from --chart-1..5   ✎ = rename (unchanged behavior)
```

The `▌` bar is the color key (replacing `lib/speaker-colors.ts`'s hardcoded
`text-emerald-600` / `bg-violet-600` set with `--chart-1..5`, cycling). Adding
**share-of-talk** is nearly free — the data is already in the transcript — and it is the
single most useful thing a meeting recorder can show you that it currently doesn't. `CH 1` is
always the owner mic (`specs/0046`), which finally makes the owner/remote boundary visible in
the UI rather than only in the diarizer. All rename/merge/assign behavior from `SpeakerLegend`
is preserved unchanged; only the container changes.

### 3.6 The lamp row and the queue — `components/Transport/LampRow.tsx`, `QueuePanel.tsx`

`LlmActivityRow` (`components/LlmActivity/LlmActivityRow.tsx`, mounted at
`Sidebar/index.tsx:209`) becomes a **three-lamp strip** at the foot of the sidebar:

```
 ┌──────────────────────────────┐
 │  ◉ READY   ○ RUN   ○ FAIL    │     all dim → idle
 │  ○ READY   ◉ RUN   ○ FAIL    │     amber, 120ms attack / 400ms decay
 │  ○ READY   ○ RUN   ◉ FAIL    │     red, latched until dismissed
 │            Summarizing…      │     ← Archivo Narrow 10px, --engrave
 └──────────────────────────────┘
```

Lamps are `READY`/`RUN`/`FAIL` mapping exactly to the existing three states at
`LlmActivityRow.tsx:27-31` — no behavior change, no new events, and the `skipped` outcome
still stays neutral (`LlmActivityPopover.tsx:57-62`). It replaces a spinner, which is the
generic-SaaS tell.

The deferred backlog (`DeferredBacklog/BacklogDetailPopover.tsx`) becomes the **QUEUE** — a
list of reels waiting for the head, right zone of the docked transport rail, with the six
existing statuses (`STATUS_LABEL`, `BacklogDetailPopover.tsx:7-14`) rendered as engraved caps
(`WAITING · TRANSCRIBING · SPEAKERS · SUMMARIZING · DONE · RETRY`) and a per-row progress
ladder instead of a spinner. `ProcessMeetingsButton` in the Today header becomes an engraved
`PROCESS 3` key.

---

## 4. Screen-by-screen

Global changes that apply everywhere, listed once: page headers unify on one formula
(`px-7 pt-7 pb-4`, Archivo 22px/600 title + `.u-meta` sub) — today there are three
(Today `px-[30px] pt-[22px]`, Meetings/People `px-8 pt-8`, Settings no sub). Content max-width
unifies at **840px** for documents and **1080px** for grids (today: 780 / 3xl / 5xl / 6xl /
1040px). Tabs unify on `MeetingTabsBar`'s 2px underline — the framer-motion spring underline in
`app/settings/page.tsx:98-103` and the two different segmented controls
(`Today/TodayToolbar.tsx:116-136` vs `app/meetings/page.tsx:341-362`) all collapse into it.

### Today (`app/page.tsx` + `components/Today/*`)

Keeps the hour-grid timeline (`DayTimeline.tsx`) — it is the right model and it is already
built on pure geometry in `lib/today-timeline.ts`. What changes: the timeline reads as a **tape
transport log**. The hour gutter becomes engraved Archivo Narrow tick marks; the "now" line
becomes a 1.5px amber rule with a 4px square (not round) index knob; `TimelineBlock`'s six
states (`TimelineBlock.tsx:17-32`) lose their `rounded-xl` + colored glow shadows and become
1px-bordered panels with a **3px left color bar** carrying the state (record red = recording,
amber = joinable, `--border` = scheduled, dashed = unrecorded). `StateChip`'s `rounded-full`
pills become engraved caps.

```
┌──────────┬──────────────────────────────────────────────────────────────────────┐
│▐ NIXON   │  Good morning, Brian                        [⌘K] [ASK] [NOTE] [● REC]│
│  ────────│  Wednesday, June 25 · 5 meetings · 2 recorded                        │
│  ● HOME  │──────────────────────────────────────────────────────────────────────│
│  ○ MEET… │  ‹  JUNE 25  ›   [📅]  [TODAY]              [DAY|WEEK]  ALL MEETINGS›│
│  ○ TASKS │                                                                      │
│  ○ PEOPLE│   09 ─┼──────────────────────────────────────────────────────────    │
│  ○ ASK   │       │ ▌ Design review                       9:30 · 4 people        │
│          │   10 ─┼─────────────────────────────────────────────────────────     │
│          │       │ ▌ Pricing sync        RECORDING  ⊙⊙   10:00 · 00:04:12       │
│  ────────│   11 ─┼──▶ now ──────────────────────────────────────────────────    │
│ ○ ◉ ○    │       │ ┆ Standup (not recorded)               11:15                 │
│ RUN      │   12 ─┼──────────────────────────────────────────────────────────    │
│ [SETTINGS│                                                                      │
├──────────┴──────────────────────────────────────────────────────────────────────┤
│ ⊙⊙ Pricing sync   00:04:12  ▁▃▅▇  │ [●REC][❙❙HOLD][■STOP] │ QUEUE 2  ◉RUN       │
└─────────────────────────────────────────────────────────────────────────────────┘
   ▐ = 6px walnut cheek          the docked transport rail is flush, full-width
```

The sidebar becomes the machine's left cheek: walnut strip, brushed `--panel` face, `NIXON`
engraved at 13px/0.18em, nav as engraved caps with a 1.5px amber index bar (not a dot) on the
active row. Widths stay 64/256 so `MainContent`'s `ml-16`/`ml-64` lockstep is untouched. The
`"V"` avatar puck (`Sidebar/index.tsx:196`) is deleted — a machine has no user avatar.

### Record — live (`app/record/page.tsx`)

The one screen that must read as a deck. Two-pane split (transcript left, notes/prep rail
right) is unchanged from `specs/0007`; the header becomes the **control panel**.

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│ ‹  Pricing sync ✎                                    TEMPLATE▾  LIVE▾  4 PEOPLE  │
│                                                                                  │
│  ⊙⊙   0:04:12     ‑20 ‑10 ‑7 ‑5 ‑3 0 +3     ┌────┐┌────┐┌────┐                 │
│  REELS  COUNTER    ╲__ CH1 MIC  ◉MIC-OK     │ ●  ││ ❙❙ ││ ■  │                 │
│                    ╲__ CH2 SYS  ▪PEAK       │REC ││HOLD││STOP│                 │
│                         LEVEL                └────┘└────┘└────┘                 │
├───────────────────────────────────────────┬──────────────────────────────────────┤
│ TRANSCRIPT                    COPY  ▾     │ NOTES │ PREP                         │
│                                           │──────────────────────────────────────│
│ 1▌ You        00:01:12                    │ AGENDA — what you planned to cover   │
│    …so on the enterprise tier we'd…       │  · volume discount                   │
│                                           │  · SSO timeline                      │
│ 2▌ Sarah Chen 00:01:20                    │──────────────────────────────────────│
│    We'd need SSO before signing.          │  - they want SSO ←                   │
│                                           │  |                                   │
│ 3▌ Speaker 3  00:01:44                    │                                      │
│    Let's target end of quarter.           │                                      │
│                                           │                                      │
│ ▌ ON THE REEL — listening                 │  autosaved                           │
└───────────────────────────────────────────┴──────────────────────────────────────┘
```

Changes: the status pill (`RecordingHeader.tsx:241-256`) is deleted — reels + lit REC key +
running counter say it better and in three places at once. The 28-bar spectrometer
(`RecordingHeader.tsx:46-78`) is replaced by the two VU meters. The `motion.div` y:20 page
entrance (`record/page.tsx:134`) goes. The transcript gutter gains the `CH n ▌` channel bar,
so live provisional speakers are legible in the same vocabulary as the post-meeting channel
strip — replacing the apologetic one-liner at `app/_components/TranscriptPanel.tsx:148-152`
with `PROVISIONAL` engraved above the list. The amber backpressure banner
(`TranscriptPanel.tsx:156-162`, currently raw `bg-amber-50` and broken in dark mode) becomes a
`DROPOUT` lamp beside the meters plus a token-colored inline rule.

### Meeting details (`app/meeting-details/page-content.tsx`)

The document screen, and the place to *stop* being a tape deck. Single-column layout, sticky
tabs, and the identity header all stay (`page-content.tsx:172-288`). The deck vocabulary
appears only at the edges.

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│ ‹  Pricing sync ✎                                                        ⋯       │
│    REEL 0412 · TUE JUN 24 · 10:00 · 00:42:18 · ZOOM                              │
│                                                                                  │
│    CH  SPEAKER          TIME    SHARE                                            │
│    1▌  You              24:10   ███████████████░░░░░  58%                        │
│    2▌  Sarah Chen       11:32   ███████░░░░░░░░░░░░░  27%                        │
│    3▌  Ravi Patel  ✎    06:36   ████░░░░░░░░░░░░░░░░  15%                        │
│  ────────────────────────────────────────────────────────────────────────────    │
│   SUMMARY │ TRANSCRIPT │ MY NOTES                     [GENERATE▾][MODEL][COPY]   │
│  ══════════                                                                      │
│                                                                                  │
│   Decisions                                                                      │
│   Ship the new editor behind a flag; revisit at end of quarter.                   │
│                                                                                  │
│   Action items                                                                   │
│   ▢ Sam — draft the pricing doc            due Fri                               │
│   ▢ Brian — confirm SSO scope              due Mon                               │
│                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────┘
```

The channel strip replaces the chip cloud and sits above the tabs (it is meeting *identity*,
not transcript chrome, and it is useful on the Summary tab too). `REEL 0412` is the meeting's
sequence number in engraved caps — the one place the metaphor touches content, and it earns it
by being a stable human-quotable handle ("pull up reel 412"). The body stays IBM Plex Sans on
`--card`. Two things must be fixed here regardless of aesthetics: `theme="light"` is hardcoded
at `components/AISummary/BlockNoteSummaryView.tsx:274` (the dark-mode break in the summary
document), and `.summary-doc`'s `--bn-colors-editor-background: transparent` needs a dark
counterpart.

### Meetings list, People, Settings, Onboarding, tray

- **Meetings** (`app/meetings/page.tsx`): list mode becomes a **tape log** — monospaced reel
  number, title, duration in counter digits, right-aligned time, 1px rules between rows, no
  card radius. The `DOT_CLASSES` rotation (`:47`) becomes the 3px channel bar. Month mode
  keeps its `gap-px over bg-border` hairline grid — which is already the most Rams-ian thing in
  the app — and only re-colors; the tri-state chips (`MonthCalendar.tsx:217-232`) become
  engraved caps.
- **People**: unchanged structurally. The star's hardcoded `text-amber-500` becomes
  `--brand`; `PersonAvatar` initials go Archivo 600 on `--chart-n`. The `MicOff` badge becomes
  a dim `NO VOICE` engraved tag, which is clearer than an icon.
- **Settings**: seven tabs stay; the framer-motion underline is replaced by the standard 2px
  CSS one; setting rows (`flex items-center justify-between p-4 border rounded-lg`) become 1px
  ruled rows with the label engraved and the `Switch` restyled as a **2-position toggle**
  (a 32×18 slot with a 14px square shuttle, no capsule). Add a **DECK / FACEPLATE** theme
  selector in General — three options (Deck, Faceplate, System), defaulting to Deck.
- **Onboarding**: the four steps stay. The 4xl serif title (`OnboardingContainer.tsx:88`)
  drops to Archivo 30px/600. `ProgressIndicator`'s circles become four engraved position
  markers with a lit amber index. `PermissionRow`'s `rounded-2xl` and raw
  `green-600`/`red-300`/`yellow-400` (`StatusIndicator.tsx:12-17`) all move to tokens. Welcome
  copy: *"Nixon records what you hear, on this Mac. Nothing is uploaded."*
- **Tray** (`src-tauri/src/tray.rs`): the emoji labels (`"⏸ Pause Recording"`,
  `"🔄 Starting Recording..."`, `build_menu:314-411`) are replaced by plain text, and the
  static `app.default_window_icon()` (`tray.rs:26`) is replaced by a **monochrome template
  icon that reflects state** — two open hubs (idle), hubs + filled center (recording), two
  vertical bars (paused). This is the cheapest high-visibility win in the whole rebrand: the
  menu bar is where the machine lives when the window is closed.

---

## 5. App icon + wordmark

**Icon: the two-reel silhouette, head-on.** Not a tape head (illegible below 64px), not a
cassette (wrong decade, and it is the *cassette* that reads as kitsch), not a caricature.

- **512px** — charcoal `hsl(195 6% 11%)` squircle; two cream hubs at 34% of the canvas
  width, centre-to-centre 46%; three tapered spoke slots per hub, rotated 60° between them;
  a 6px cream tape path running hub-to-hub, dipping 8px at centre over a dark head block; a
  single 14px amber dot at the lower-left as the REC lamp. A 1px cream top bevel on the
  squircle.
- **128px** — drop the head block and the bevel; keep two hubs, spokes, tape path, amber dot.
- **32px** — two solid cream rings (no spokes) + the tape path as a 2px bar. No lamp.
- **16px** — one 2px cream horizontal bar with a cream dot at each end: the `⊙—⊙` silhouette.
  It must survive being 16px and greyscale in a Finder list, and this does.

The current icon (`frontend/src-tauri/icons/128x128.png` — cream ground, script "vinyl"
logotype, orange record and tonearm) is replaced entirely; the whole `icons/` set plus a new
monochrome `tray-template.png` / `@2x`.

**Wordmark: `NIXON`**, Archivo 600, all caps, `letter-spacing: .18em`, engraved treatment
(`--engrave` fill + the 1px bevel text-shadow from §2). Sidebar at 13px/.18em; About at
28px/.22em with a 1px `--border` rule 10px beneath it, full width of the panel — the way a
model number sits under a brand on a faceplate. No logotype, no ligature, no reversed letter,
no icon lockup. The name is set in the same face as the panel labels because on a real machine
it *is* a panel label.

---

## 6. What NOT to do

The kitsch line, concretely:

- **No photographic or generated textures.** No wood-grain image, no brushed-metal bitmap, no
  paper/leather. The walnut cheek is 6px of flat color; if it needs grain, delete it.
- **No rendered hardware.** No screws, no vents, no rack ears, no drop-shadowed knobs, no
  radial-gradient "metal" bezels, no glass reflections on the VU face.
- **No rotary knobs as controls.** A knob is a terrible mouse target and a worse keyboard
  target. Rotary *forms* are allowed only where nothing is adjusted (the reels). Everything
  adjustable is a key, a toggle, or a slider.
- **No audio.** No click on press, no relay clack on stop, no tape hiss. Ever.
- **No fake wear.** No scuffs, no scratched anodizing, no "vintage" vignette, no grain that is
  actually visible.
- **No blinking as a persistent state.** Blink is an *alert*; a 90-minute recording is not an
  alert. (This deletes `grb-blink`, `animate-pulse`, and `animate-ping` from the recording
  chrome.)
- **No Nixon iconography.** No likeness, no signature, no seal, no "18½ minutes" progress bar,
  no Watergate references in UI copy. The name is the whole joke.
- **No serif.** Newsreader leaves; a serif display face on a machine panel is the tell.
- **No new metaphor nouns for content.** See §1.

### Accessibility guardrails

- **Contrast is checked, not assumed.** Every pair in §2 is computed: cream on chassis 13.5:1,
  `--muted-foreground` 5.3:1 dark / 4.6:1 light, amber 8:1. `--record` is **4.6:1 and therefore
  never text on the deck** — REC is cream-on-lit-key.
- **State is never color alone.** Every lamp has an engraved caps label beside it
  (`REC`, `HOLD`, `MIC`, `RUN`, `FAIL`). This is also why the lamp row beats the spinner.
- **`prefers-reduced-motion`** stops the reels (static + lit lamp), pins the needle to its
  instantaneous value with no ballistics animation, disables the counter digit roll, and
  removes all route transitions. The recording state must remain fully readable with every
  animation off — test by forcing it.
- **`prefers-reduced-transparency`** disables the grain overlay and the lamp halos.
- **Minimum sizes:** engraved caps never below 10px (they are already tracked +.14em, which
  costs legibility); body text never below 13px; the transcript/summary body stays 14px.
  Transport keys are 44×34 — above the 44px touch dimension on the long axis and comfortably
  above macOS pointer minimums.
- **Focus is visible on a dark deck**: 2px `--ring` amber outline with a 1px `--background`
  offset, on every key. Amber at 8:1 satisfies the 3:1 non-text requirement with margin.

---

## 7. Rollout

Effort: **S** ≈ <1 day, **M** ≈ 1–3 days, **L** ≈ multi-day.

### Phase A — mechanical rename (M)

**Recommendation: keep the bundle identifier `ai.vinyl.app` / `ai.vinyl.app.debug`.** The
identifier drives the app-data dir, the single-instance lock, the TCC grants (mic +
screen-recording), the Keychain ACLs (ADR-0009), and the EventKit/Google calendar grants.
Changing it re-prompts for **every** permission on the user's real install, re-exercises
`src-tauri/src/data_migration.rs` for a second time, and buys nothing a user can see —
identifiers are invisible. `~/Movies/meetily-recordings/` and the `vinyl-vX.Y.Z` release tag
prefix should likewise stay (both are functional, both were already deliberately kept in
`specs/0002` task 3). If the owner overrules this, the migration is marker-guarded and
non-destructive, but budget a full `docs/MANUAL_SMOKE.md` pass plus a permission re-grant
walkthrough.

Surfaces to change:

| Surface | Anchor |
|---|---|
| `APP_NAME` (non-Tauri fallback path) | `frontend/src-tauri/src/app_paths.rs:21` |
| productName + window title | `frontend/src-tauri/tauri.conf.json:3`, `:15` |
| npm package / crate / binary | `frontend/package.json:2`, `frontend/src-tauri/Cargo.toml` — **own commit**; touches `src-tauri/scripts/cargo-dev-sign.sh` (`target/debug/vinyl`), `dev-vinyl.sh`, `build-dev-app.sh`, `upgrade-vinyl.sh`, `clean_run.sh`, `release.sh`, `scripts/tauri-auto.js` |
| Page metadata | `frontend/src/app/metadata.ts:4`, `metadata.tsx:4` |
| Sidebar wordmark | `components/Sidebar/index.tsx:100` |
| About | `components/About.tsx:16` |
| Product-name copy (≈22 strings) | `TranscriptEmptyState.tsx:51`, `PermissionWarning.tsx:101,127`, `PermissionsModal.tsx:187`, `RecordingPermissionsSettings.tsx:83`, `ZoomMuteGateToggle.tsx:38`, `CalendarSettings.tsx` ×8, `RecordingSettings.tsx:709,719,751`, `OwnerEmailSettings.tsx:100`, `ZoomAutoDetect.tsx:178`, `person-details/page.tsx:616`, `UnprocessedTranscriptEmptyState.tsx:10` |
| Icons + tray template | `frontend/src-tauri/icons/*`, `tray.rs:26` |
| Docs | `README.md`, `CLAUDE.md`, `SETUP.md`, `PRIVACY_POLICY.md`, `CONTRIBUTING.md`, `ROADMAP.md`, `docs/MANUAL_SMOKE.md`, a new `CHANGELOG` entry (do not rewrite history) |

`DevBadge` keeps saying `DEV`. Three test assertions reference the product name
(`components/__tests__/CalendarSettings.test.tsx:41,110,128`) and will need updating.

### Phase B — tokens + fonts (S–M) ← the cheapest big win

`globals.css` token blocks replaced wholesale (§2), `.dark` promoted to the default via
`<html className="dark">` in `app/layout.tsx:248` plus a Settings theme selector; fonts swapped
at `layout.tsx:43-54`; `tailwind.config.js` gains `panel`, `engrave`, `success`, and
`meter-over` (and loses the stray `tertiary: '#64748b'`); `--radius` 0.75rem → 0.25rem;
`.u-section-label` / `.u-doc-heading` / `.u-meta` bodies rewritten (names kept, so no call
sites move); `animate-vibrate` / `fade-in-up` keyframes and the hardcoded `#d1d5db` scrollbar
colors deleted.

**Phase B is not done until the 285 off-token classes are swept**, or dark mode ships broken.
The must-fix list: `lib/speaker-colors.ts:12-21,47-56` → `--chart-1..5`;
`BlockNoteSummaryView.tsx:274` `theme="light"` → theme-aware; the amber banners
(`app/_components/TranscriptPanel.tsx:156`, `SummaryPanel.tsx:369`); the red device-error alert
(`RecordingControls.tsx:494`); `DevBadge.tsx:24,34`; `AudioLevelMeter.tsx:34,66`;
`TimelineBlock.tsx` `bg-emerald-50 text-emerald-700`; `app/people/page.tsx:147` amber star;
onboarding `StatusIndicator.tsx:12-17` and `ProgressIndicator.tsx` `green-600`;
`components/ui/button.tsx:22`'s `green` variant (hardcoded `bg-green-700`) — and while in there, rename the
lying variants `blue`→`brand` and `red`→`record`.

### Phase C — signature components (M–L)

The six of §3, each a new file under `components/Transport/` (plus `ChannelStrip`). Delete
`components/RecordingStatusBar.tsx` (dead — no mount anywhere), the `showPlayback` branch of
`RecordingControls.tsx:350-385` (vestigial, all values hardcoded `0`), the unused `chrome`
variant of `MeetingIdentityHeader.tsx:254-267`, and the commented-out toolbar at
`AISummary/index.tsx:669-760`. Wire `recording-level` for the first time. One small Rust
change: emit a mute-gate state event so the `MIC` lamp has a source.

### Phase D — per-screen polish (L)

§4, screen by screen, in the `specs/0014` wave style: Today → Meetings → Meeting details →
People → Settings → Onboarding. Each wave gated on `pnpm lint` + `pnpm test` + `tsc` + a
launch, reviewed by eye before the next.

### Risks

1. **File-size ratchet** (`scripts/check-file-size.sh`, 800-line limit, allowlisted files may
   only shrink). Every signature component must be a *new* file; do not grow
   `RecordingControls.tsx` (524) or `SpeakerLegend.tsx` (~700). Phase C should *reduce* both.
2. **`cargo fmt` + the ratchet** — the known trap: a bare `cargo fmt` reflows and breaks it.
3. **Test churn is small but real.** There are **no snapshot files** in `frontend/src`, so the
   exposure is limited to tests asserting on visible strings — the three `CalendarSettings`
   assertions above, and any test matching status text changed by §3's state vocabulary
   (`app/meetings/__tests__/page.test.tsx`, `app/meeting-details/__tests__/*`). Renaming
   `Pause`→`HOLD` visually while keeping the `aria-label` protects the query-by-role tests.
4. **Dark-mode parity is the top functional risk**, not an aesthetic one. Flipping the default
   to `.dark` with 285 hardcoded light-palette classes still in the tree produces white-on-white
   banners and invisible chips. Phase B's sweep is load-bearing; do not ship B without it.
5. **Needle ballistics on a 12.5 Hz feed** must be verified against real audio, not a mock —
   if it reads sluggish, the fix is the integrator constant, not a faster event (raising the
   emit rate touches the hot recording loop).
6. **No GUI verification in the agent environment.** Every phase needs an owner smoke pass, and
   the `specs/0054` lesson (shipped with no human smoke) applies with force to a pure-visual
   change.

---

## 8. Open questions for the owner — answered, see "Decisions locked"

1. **Dark as the default?** I argue yes (§2) with light at full parity and a Settings
   selector. If you want light primary, the palette inverts cleanly but the lamps lose most of
   their force and §3 gets noticeably less interesting.
2. **Keep `ai.vinyl.app` as the bundle identifier?** I strongly recommend yes (§7): invisible
   to users, and changing it re-prompts every macOS permission on your real install.
3. **How far does the metaphor go into the content nouns?** My recommendation is "not at all,
   except `REEL <n>` on the meeting header." If you want Meetings → *Reels* app-wide, say so
   now — it changes nav, empty states, search copy, and the docs.
4. **Needle VU, or segmented ladder?** The needle is the identity move and costs a real
   afternoon of ballistics work. The ladder is a day cheaper and 80% as good.
5. **Walnut cheek: in or out?** It is the one warm, decorative, non-functional element in the
   whole design. It is also the one most likely to read as costume.
6. **One big branch or the `specs/0014` wave pattern?** Waves are safer and let you veto per
   screen, but the app will look half-rebranded for a couple of weeks.

## Plan 1 residuals (2026-09-13) — carry into Plan 2 / Plan 3

Plan 1 (`docs/superpowers/plans/2026-09-13-0057-nixon-foundation.md`, Phases A+B) landed on
`feat/0057-nixon-rebrand` at `28e4940`. Items parked by the final review, in priority order:

**Plan 2, first task:** the recordings write root is a filesystem probe
(`audio/recording_preferences.rs::get_default_recordings_folder`, 6 call sites) while
`fs_guard`, folder delete and "Open recordings folder" read the persisted `save_folder`. The
Plan 1 sticky-legacy rule (legacy `meetily-recordings` wins while it exists) keeps the owner's
install consistent but mis-handles the SETUP.md "rsync to a new machine" case (fresh install +
copied legacy folder → writes move outside the allowed roots). Durable fix: the write path reads
`save_folder`, probing only when no preference is stored. Same task: the `.env.google` warning in
`dev-nixon.sh` / `upgrade-nixon.sh` should also check `NIXON_GOOGLE_CLIENT_SECRET`.

**Plan 2:** AudioLevelMeter over-zone uses `record` (a `meter-over` token exists) and its 0.5
zone edge is uncalibrated — the VU replacement owns this; ChunkProgressDisplay "Paused" badge
shares `brand` with processing (queue/lamp redesign); add a comment on `--bevel-hi/-lo` (alpha
carrying, consume as `hsl(var(--x))`, never register in tailwind colors) when the bevels are first
used; tray template icons by state (spec §4).

**Plan 3:** `DOT_CLASSES` in `app/people/page.tsx` and `app/meetings/page.tsx` plus
`person-details` `AVATAR_CLASS` still use a 4-entry chart rotation (transcript speakers now use
8 slots); sidebar "N" puck deletion (spec §4); Appearance radiogroup arrow-key roving; hydration
warning on `aria-checked` when a persisted theme differs from the SSR default; gate blind spots
(`bg-[rgb(…)]`, files outside `frontend/src`); `tsc --noEmit` cannot be a CI gate until
`tests/lib/blocknote-markdown.test.ts` (`bun:test`) is excluded or ported; TemplateSettings
`loadError` whole-body destructive; `.design-sync/*` "Vinyl UI" / `window.VinylUI` tooling
identifiers; dated specs (0043/0047/0048/…) still document `VINYL_*` env vars as runbooks — add a
pointer in `specs/INDEX.md`.

**Owner design call:** the app icon (two reels over an arched tape with a head block) can read as
a frowning face at 32/512px; dipping the tape arc or widening the head block fixes it.

## Plan 2 residuals (2026-09-13) — carry into Plan 3

Plan 2 (`docs/superpowers/plans/2026-09-13-0057-nixon-signature-components.md`, Phase C) landed
on `feat/0057-nixon-rebrand`. Parked items, grouped:

**Spec deviations accepted this plan (own them in Plan 3):**
- Per-channel VU meters — spec §3.2 asks for two; the backend emits one mixed `recording-level`,
  so the shipped meter is mixed. Needs per-channel rms out of
  `frontend/src-tauri/src/audio/pipeline.rs`.
- DROPOUT lamp — deferred; wire `transcriptGaps` from
  `frontend/src/app/_components/TranscriptPanel.tsx:111-123` into the rail/record lamps.
- Channel strip relocation above the tabs (page-content, not inside the Transcript tab) —
  per-screen work; `frontend/src/components/MeetingDetails/SpeakerLegend.tsx`.
- `REEL nnnn` ordinal on a meeting — no DB column; derive
  `ROW_NUMBER() OVER (ORDER BY created_at)` in `api_get_meetings`
  (`frontend/src-tauri/src/meetings/`).
- Tray template icons by state (spec §4) — still one static icon.

**Correctness / hygiene (from the task ledger):**
- Make `get_default_recordings_folder` private/non-re-exported (and `RecordingsRootCache`
  `pub(crate)`) so a regression to the filesystem probe fails to compile —
  `frontend/src-tauri/src/audio/recording_preferences.rs`.
- `frontend/src/hooks/useDeferredBacklog.ts:137` still has its own fetch-all loop — fold into
  the shared `frontend/src/lib/fetch-all-transcripts.ts` helper.
- Talk-time map dips to the paginated estimate on refetch after speaker edits — keep the last
  good map while the full fetch is in flight (`useMeetingTalkTime`).
- Add a hook race test for the talk-time fetch-all (out-of-order resolution).
- `isRecordingDisabled` is write-only (`frontend/src/hooks/useRecordingStateSync.ts:23`) and
  `handleRecordingStart` (`frontend/src/hooks/useRecordingStart.ts`) has no production caller —
  a near-duplicate of the sidebar listener; collapse both.
- `transcript-error` listener has no Rust emitter (dead path) — remove or emit it.
- `errorAlert` title/body is a string wire convention — a structured payload would be durable.
- `DOT_CLASSES` avatar rotation is still 4 entries while transcript speakers use the 8-slot
  palette — `frontend/src/app/people/page.tsx`, `frontend/src/app/meetings/page.tsx`,
  `person-details` `AVATAR_CLASS`.
- Tests not written this plan: a `VuMeter` component test covering the rAF `dt` clamp after a
  background gap (`frontend/src/components/Transport/VuMeter.tsx`).
- The `recording-spectrum` backend emit is now unconsumed
  (`frontend/src-tauri/src/audio/pipeline.rs:1087`) — remove it or keep it deliberately.
- Node 25 locally vs the `engines` range in `frontend/package.json` — pick one.

## Plan 3 residuals (2026-09-14) — carry forward

Plan 3 (this doc's Phase D — chrome, sidebar, reel label + typed transcript, tape logs, tray
icons, geometry pass) landed on `feat/0057-nixon-rebrand`. Task 9 fixed what it touched
directly (pill sweep, walnut-cheek z-index, `ReelLabel` `aria-hidden` + shadow, tray
`set_icon` error logging, the `tasks/page.tsx` status filter → `SegmentedControl`, the
meetings test's framer-motion mock + `localStorage.clear()`); the rest carries forward:

**Spec deviations still open (own them in the next plan):**
- ~~Per-channel VU meters~~ — DONE post-Plan 3 (2026-09-14): `audio/live_meter.rs` emits
  `mic` / `sys` pairs on `recording-level`; the record header shows CH1 MIC / CH2 SYS.
- The DROPOUT lamp (carried from Plan 2 — see above): `frontend/src/app/_components/TranscriptPanel.tsx`
  still doesn't feed `transcriptGaps` into a rail/record lamp.
- `REEL nnnn` is still a **derived** ordinal (`ROW_NUMBER() OVER (ORDER BY created_at)` in
  `api_get_meetings`), not a stored column — deleting an older meeting renumbers later reels.
  Accepted for Plan 3; a stored `reel_number` column assigned at creation (migration +
  backfill, so a deletion can't silently relabel someone else's tape) is a follow-up plan's
  first task if the renumbering-on-delete behavior proves confusing in the field.
- ~~`ReelLabel` `--paper-rule` token / `deck` / `tags` wiring~~ — MOOT (2026-09-15): the card was
  removed on owner canvas feedback (too tall, redundant with the identity line); the
  identity line now carries the voice count too.
- `handleRecordingStart` (`frontend/src/hooks/useRecordingStart.ts`, 812 lines) still has no
  production caller — a near-duplicate of the sidebar listener. Ruling stands: leave in place
  (resume tests exercise the shared start core); collapsing it into the sidebar listener is
  still open.
- The `recording-spectrum` backend emit (now `frontend/src-tauri/src/audio/live_meter.rs`) is
  still unconsumed by the frontend — remove it or wire it to something rather than leaving a
  live, unused event. (Kept through the per-channel VU work so the pipeline integration
  tests' spectrum contract stayed untouched.)
- `frontend/package.json`'s `engines` range still doesn't match the local Node 25 toolchain —
  pick one and reconcile.
- `TapeCounter` (used per-row in the Meetings tape log and Today timeline) is a full animated
  reel component; in a 100+ row list that's roughly 7k DOM nodes just for counters. Not shown
  to be slow yet — measure a large list before investing in a static-text list variant.
- `app/tasks/page.tsx`'s `PageHeader` `<h1>` still has no `truncate` (pre-existing shape,
  shared by every `PageHeader` caller — fixing it belongs with a `PageHeader` pass, not a
  single-page patch).
- `DownloadProgressStep` / `SetupOverviewStep` onboarding re-skins still have no automated
  tests — covered only by the fresh-profile manual smoke walk (`docs/MANUAL_SMOKE.md` §18).
