# ClicKS

Real-time timing, metronome, SMPTE LTC and playback engine for live theatre and concert productions.

`clicks-core` is the runtime component of the ClicKS system. It runs on stage hardware and acts as the authoritative tempo and cue processor during a performance.

---

## Scope

Provides:

- Deterministic tempo engine
- Cue-based (non-strictly-linear) execution model
- Real-time tempo changes
- Vamp / adaptive sections
- SMPTE LTC generation
- Multi-channel audio playback via JACK
- Live network monitoring using binary control protocol

Designed for embedded or headless Linux systems (e.g. Raspberry Pi).

---

## Related Components

- [`clicks-editor`](https://github.com/lexag/clicks-editor) — show programming
- [`clicks-monitor`](https://github.com/lexag/clicks-monitor) — live monitoring and control
- [`clicks-common`](https://github.com/lexag/clicks-common) — shared communication formats and protocol definitions

---

## Architecture

- Audio processor
  - Metronome
  - SMPTE LTC generation
  - 30 channels of audio playback
- Network handler
  - Lightweight binary protocol i/o
  - JSON i/o
  - OSC i/o

Execution is cue-based and supports non-linear flow.

---

## Build

Requirements:

- Rust (stable)
- Linux
- JACK

Build:

```bash
cargo build --release
```

Run:
```bash
cargo run
```

Prebuilt binaries are available in Releases.

## Deployment

Intended to run:
- On headless systems
- As a system service (recommended)
- Automatic JACK server and client startup, no setup needed
- With automatic restart on failure

## Show Data
- Primary format: compact binary
- JSON export/import supported (via clicks-editor)
- Protocol and format definitions in clicks-common

## Design Constraints
- Deterministic timing
- Low runtime overhead
- Headless operation
- Minimal dependencies (JACK required)
- Extensible protocol layer


## Roadmap
- More live protocols (OSC, MIDI, DMX)
- Platform agnostic audio handling (JACK, ALSA, ASIO)
