# Provenance

This project reimplements control of a commercially sold instrument from public
documentation and from observing the instrument. No vendor source code, binary code,
data files or assets are included, copied, linked or read at build time.

## Sources

| Fact class | Source | Evidence |
| --- | --- | --- |
| POLY D SysEx framing, packet numbers, payload shapes, ranges, defaults | Behringer POLY D Quick Start Guide, "POLY D System Exclusive Data Sheet", p. 82-85 | `QSG` |
| POLY D ACK codes, settings read (`0x75` → `0x76`) and the 25-byte block layout, packets absent from the QSG (`0x1C`, `0x2F`), rejected packets, framing acceptance | Measurements on a POLY D running firmware 1.1.3 | `verified` |
| LM DRUM CC map (filter cutoff, 16 voice tunings) | Behringer LM DRUM user manual p. 27 | `QSG` |
| LM DRUM settings read, 108-byte block layout, note map, channel, routing/switch frames, trigger-output value domain, packet-to-offset mapping | Measurements on an LM DRUM running firmware 1.1.7; trigger-output voice order cross-checked against manual p. 24 defaults | `verified` / `vendor-named` |
| Envelope rules for protocol families, opcodes for mode/firmware/restore | Same measurements plus the MIDI System Exclusive specification | `verified` |

Transcripts: [`verified/poly-d-measurements.md`](verified/poly-d-measurements.md),
[`verified/lm-drum-measurements.md`](verified/lm-drum-measurements.md).
Machine-readable device facts: [`verified/poly-d.toml`](verified/poly-d.toml),
[`verified/lm-drum.toml`](verified/lm-drum.toml).

## What is not used

- No SynthTribe/`Synthtool` binary, resource, `config.json`, SQLite database or image is
  shipped, parsed, embedded or downloaded by this repository.
- `cargo run -p xtask` reads only the hand-written overlays under `verified/`. Earlier drafts
  used the vendor configuration file to seed parameter titles and read-back offsets; that
  dependency was removed. Every read-back offset has since been measured independently
  (write a distinct value, read the block, diff), and parameter names are either the
  documentation's own wording or neutral functional names. For the LM DRUM, which has no
  published SysEx packet table, only measured packets and documented CCs are exposed. Its
  remaining settings carry the `vendor-named` tag: the wire address and value domain are
  measured, while the human name comes from the vendor configuration. Names never upgrade a
  packet to "verified" on their own.
- UI text, colours, icons and layout are original. The screenshot in `docs/` is this
  application.
- No code was decompiled, disassembled or transcribed. The protocol is exercised over
  MIDI SysEx, which is an interoperability interface, using the vendor's published table.

## Dependencies

All dependencies are permissively licensed and unmodified: `eframe`/`egui` (MIT OR
Apache-2.0), `midir` (MIT), `serde`, `serde_json`, `toml` (MIT OR Apache-2.0). No vendored
or patched third-party code.

## Names and marks

"Behringer", "POLY D" and "Music Tribe" are trademarks of their owners. They appear only to
identify the hardware this tool talks to. The project name is `opentribe`; it is not
affiliated with or endorsed by the manufacturer.

## Reproducing the measurements

The helper scripts are not part of the build. Each entry in the transcript records the exact
request bytes and the reply bytes, so any owner of the instrument can re-run them with
`amidi`/`rawmidi` or the app's Console tab and compare. `verified/poly-d.toml` lists the
resulting facts, and `cargo test -p tribe-poly-d` pins the wire format against the QSG
examples and the observed block.

## A note on "clean room"

A strict clean-room process separates the person who studies the original from the person
who writes the code. This project did not run such a process: the same author read the QSG
and the instrument responses and wrote the code. What it does guarantee is that the shipped
artifact is *independent* — its content is traceable to published documentation and to
measurements of observable behaviour, contains no vendor expression, and has no build-time
dependency on vendor software or data. That is the property that matters for this kind of
interoperability tool, and it is checkable from the repository contents.
