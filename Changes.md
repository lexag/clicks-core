## v1.3.0 - 2026-03-04
- Update to ClicKS common v2.2.0
    - Send new PlaybackData and PlaybackHandlerChanged on playback frames and cue loads, respectively
    - SMPTE generator supports user bits, frame offsets and multiple frame rates
    - The RunEvent ControlAction can now be used to run arbitrary events from clients
- Fix [#16 Crash on seeking in large playback file](https://github.com/lexag/clicks-core/issues/16)

## v1.2.4 - 2026-02-26
- SMPTE LTC now goes 0-24 instead of 1-25. Customizability of this and other settings are planned for version 1.3, see [common#31](https://github.com/lexag/clicks-common/issues/31)

## v1.2.3 - 2026-02-19
- Fix SMPTE LTC not in spec
- Fix playback working on first cue after load

## v1.2.1 - 2026-01-21
- Refactor log handling
- Basic implementation of hardware controller

## v1.2.0 - 2025-12-21
- Migrate to ClicKS common v2.0.0
- Hardware integration (#4)
  - Added I2c support
  - Interactive patching and loading from USB drive for rack unit
  - Display screen and button interactivity on rack unit
- Remove local deploy script from repo
- Implement Large and Small Message
- Implement postcard binnet encoding
- Explicit message size in binnet message

## v1.0.0 - 2025-09-29
- Tiny release
