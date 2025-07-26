# torque-tracker-engine
Torque tracker is a reimplementation of [schismtracker](https://github.com/schismtracker/schismtracker).

My GUI implementation is on [crates.io](https://crates.io/crates/torque-tracker). The library should be
flexible enough to be used in multiple UI libraries and multiple audio output libraries.

Work in Progress.

## Current Features
- simple audio playback & rendering
- partial loading of schism tracker files
- per channel volume and pan
- song updates while playing

## Planned / Wanted Features
- audio effects
- "instruments" (in the way schism tracker uses that word)
- sample settings
- complete loading and storing of schism project files
- higher quality interpolation algorithms
