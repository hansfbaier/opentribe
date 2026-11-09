# PRO-800 measurements

Unit: Behringer PRO 800, USB `1397:125f`, firmware **1.4.6**, mode normal.
Measured 2026-11-09 over ALSA (`PRO 800 MIDI 1`, `hw:5,0`) with `opentribe-pro-800`.
Earlier protocol notes are in `devices/PRO-800/` in the research notebook.

## Frames observed

| Operation | Request | Reply |
| --- | --- | --- |
| mode query | `F0 00 20 32 00 01 24 00 02 F7` | `F0 00 20 32 00 01 24 00 03 00 F7` |
| firmware | `F0 00 20 32 00 01 24 00 08 00 F7` | `F0 00 20 32 00 01 24 00 09 00 01 04 06 F7` |
| global read | `F0 00 20 32 00 01 24 00 77 7E 03 F7` | `F0 00 20 32 00 01 24 00 78 7E 03 <packed 40 B> F7` |
| program read | `F0 00 20 32 00 01 24 00 77 <lsb> <msb> F7` | `F0 00 20 32 00 01 24 00 78 <lsb> <msb> <packed> F7` |
| empty slot | same read | **bare `F0 F7`** |
| program write | `F0 00 20 32 00 01 24 00 78 <lsb> <msb> <packed 173 B> F7` | `F0 00 20 32 00 01 24 00 01 00 00 F7` |
| delete | `F0 00 20 32 00 01 24 00 78 <lsb> <msb> F7` | `F0 00 20 32 00 01 24 00 01 00 00 F7`, then the slot reads `F0 F7` |

Notes:

- An empty slot answers with a zero-length SysEx frame. The 12-byte
  `78 <lsb> <msb>` header with no payload appears in bank exports produced by editors, not on
  this unit's wire.
- Writes carry the full 173-byte record. A capture whose trailing zeros were trimmed is padded
  before it is sent; the read-back then has 173 unpacked bytes.
- The first frame after opening the port can be dropped. The CLI sends a mode query on open and
  retries a silent read once.

## Validation run

1. Full read-only scan: **380 populated, 20 empty**. Empties match
   `vendor/swumpf/BANK_ABCD_20261109_1923.syx` exactly.
2. Full backup to `/tmp/pro800-live-backup` (400 `.syx` files + `manifest.csv`), 15 s at the
   30 ms pace.
3. Payload comparison against that bank export: **20 identical empty slots**, **380 populated**
   with differences only at unpacked bytes 171–172 (the earlier bulk pitch-bend write,
   `00 60` → `05 19`) and appended trailing zeros from that padded write. Two slots changed on
   the panel since the export: B098 has a blank name, B099 is named `JazzBass HB`.
4. GUI on connect: reads mode, firmware, the 40-byte global block, then loads the program named
   in the global block (B099) — `JazzBass HB`, pitch-range knob 98 for raw `0x1905`.
5. `copy --from B099 --to B094`: ACK `01 00 00`, read-back byte-identical, `diff` reports
   0 changed bytes.
6. `delete --slot B094`: ACK `01 00 00`, read-back `F0 F7`, editor cleared.
7. B099 re-read after both writes: SHA-256 unchanged
   (`9ae1564a6c53beda1caebbac4b623eaec7aefe0bc546474bf0e83b9b2af4e6a7` of the `.syx`).

Gate G1 (patch write path) is re-confirmed on this firmware with the app's own commands. Gates
G2 (per-parameter offsets read on the panel) and G3 (global write) remain open.
