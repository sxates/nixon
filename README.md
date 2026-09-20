# Nixon

**Nixon is a local-first, on-device meeting assistant for macOS.** It records your
meetings (system audio + mic — Zoom / Meet / Teams), transcribes them in real time,
summarizes them, and organizes them for later reference.

Privacy is the whole point: meeting audio, transcripts, and notes never leave your
machine. The only outbound traffic is to a **you-chosen** LLM provider for
summarization — and the default is a local model via Ollama, so everything can stay
on-device. Nixon also checks GitHub for new versions in the background (no personal
data is sent); this can be turned off in Settings.

> Nixon is a hard fork of [meetily](https://github.com/Zackriya-Solutions/meeting-minutes)
> v0.4.0 (MIT) and diverges substantially. See [`CHANGELOG.md`](CHANGELOG.md) for the
> post-fork history.

## Screenshots

| Today | Recording |
|---|---|
| ![Today view](docs/screenshots/real/today.faceplate.png) | ![Live recording with VU meters](docs/screenshots/real/record-live.faceplate.png) |

| Transcript with speakers | Summary |
|---|---|
| ![Transcript](docs/screenshots/real/meeting-transcript.faceplate.png) | ![Summary](docs/screenshots/real/meeting-summary.faceplate.png) |

| Ask AI | Settings |
|---|---|
| ![Ask AI](docs/screenshots/real/ask.faceplate.png) | ![Recording settings](docs/screenshots/real/settings-recordings.faceplate.png) |

Deck (dark) variants live in [`docs/screenshots/real`](docs/screenshots/real). Every image is rendered from the fictional demo dataset (`./dev-nixon.sh --demo`).

*Images are regenerated with `pnpm shots:real` from a terminal that has Screen Recording permission; see [`docs/screenshots/README.md`](docs/screenshots/README.md).*

## Features

- **On-device transcription** — Whisper.cpp or NVIDIA Parakeet, real-time, with
  Metal / CoreML acceleration on Apple Silicon.
- **Notes-aware AI summaries** — a live notepad beside the transcript; your notes
  ground the summary so it captures what *you* thought mattered. Runs on local
  Ollama by default, or Anthropic Claude / OpenAI / Groq / OpenRouter / any
  OpenAI-compatible endpoint.
- **Speaker diarization** — label who said what, fully on-device; rename or merge
  speakers and seed names from your calendar's attendee list.
- **Calendar + Zoom** — reads your macOS Calendar to show your whole day's agenda,
  with one-click **Join & Record**.
- **Search & organization** — a meeting dashboard with full-text search.

## Requirements

**Any Apple Silicon Mac running macOS 14.4 or later.** That includes the original M1 — the
work is sized to the machine rather than gated on a recent one. (14.4 is where the Core
Audio process tap arrives, which is what lets Nixon capture system and Zoom audio without
BlackHole. Intel Macs are not supported.)

You will also need to grant **microphone** and **audio-capture** permission, once.

### What your hardware changes

Everything — transcription, speaker identification, summarization — runs on your Mac, so
your hardware decides two things: which summary model Nixon picks, and how long you wait.

| Your Mac | Summary model | Hour-long meeting summarized in |
|---|---|---|
| 8 GB RAM | Qwen 3.5 2B (~1.8 GB in use) | about a minute on an M1, less on newer chips |
| 16 GB RAM or more | Qwen 3.5 4B (~3.9 GB in use) | seconds to half a minute |

**RAM is what unlocks the better model.** Nixon checks how much you have on first run and
picks for you — you never have to choose, though Settings → Summary will let you if you
want to. 16 GB is the line, and crossing it buys summary quality rather than speed.

**A faster chip buys time, not capability.** Every Apple Silicon generation transcribes
faster than the meeting happens, so live transcripts keep up regardless; a quicker GPU
mostly means the summary lands sooner after you stop recording.

**On an 8 GB Mac, watch what else is open.** Nixon's own footprint is modest, but it shares
memory with your video-call app — which is by definition running during a meeting.

*(Memory figures are measured; the times are estimates scaled from a measured baseline, so
treat them as the right order of magnitude rather than a benchmark.)*

### Disk

About **3 GB** for the models Nixon downloads the first time it needs them — speech
recognition ~0.6 GB, speaker identification ~0.1 GB, summarization 1.2–2.6 GB depending on
your RAM — plus roughly **250 MB per hour** of meetings you record. Recorded audio can be
set to auto-delete after a number of days in Settings → Recordings.

## Build & run (macOS / Metal)

```bash
cd frontend
pnpm install
./dev-nixon.sh      # dev build ("Dev Nixon"), isolated from production data
./build-gpu.sh      # production build → ../target/release/bundle (.app + .dmg)
./upgrade-nixon.sh  # rebuild + reinstall /Applications/Nixon.app, preserving data
```

See [`CLAUDE.md`](CLAUDE.md) for the full architecture map and build notes.

## Privacy

Meeting audio, transcripts, and notes are stored locally and never transmitted.
The only outbound traffic is to your chosen LLM provider for summarization; use the
default local Ollama to keep everything on-device. Nixon also checks GitHub for new
versions in the background (no personal data is sent); this can be turned off in
Settings. See [`PRIVACY_POLICY.md`](PRIVACY_POLICY.md).

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

MIT — see [`LICENSE.md`](LICENSE.md). Nixon is a fork of meetily (also MIT); the
original copyright notice is retained.

## Acknowledgments

- [meetily](https://github.com/Zackriya-Solutions/meeting-minutes) — the upstream
  project Nixon forks.
- Code borrowed from [Whisper.cpp](https://github.com/ggerganov/whisper.cpp),
  [Screenpipe](https://github.com/mediar-ai/screenpipe), and
  [transcribe-rs](https://crates.io/crates/transcribe-rs).
- **NVIDIA** for the **Parakeet** model, and
  [istupakov](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx) for the
  ONNX conversion.
