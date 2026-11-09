# PRO-800 app — specification

Status: M0, M1 and the GUI are implemented; G1 re-run on firmware 1.4.6 on 2026-11-09
(`verified/pro-800-measurements.md`). in `crates/pro-800` (2026-11-09); M2 and M3 remain gated on G2 and G3. Draft, 2026-10-29. Target: a `tribe-pro-800` crate plus `opentribe-pro-800` binary in this
workspace, alongside `tribe-poly-d` and `tribe-lm-drum`.

Sources: `/home/jack/reveng/behringer-synthribe` (protocol notebook). Facts below carry the same
evidence tags the notebook uses — **[HW]** measured on this instrument, **[CODE]** vendor app
disassembly, **[DATA]** shipped files, **[MAN]** manual, **[INF]** inference. Anything without a
tag in this document is a design decision, not a claim about the device.

---

## 1. Scope

**In scope**

1. Patch librarian for all 400 slots (4 banks × 100): dump, store, rename, copy, backup, restore.
2. Per-parameter patch editing **only for offsets that pass the verification gate (§11 G2)**.
3. Global settings: read all 40 bytes; write **only** the bytes that pass G3.
4. GUI, CLI, JSON and MCP surfaces over the same descriptor, as the other two devices have.

**Out of scope**

1. Reading the edit buffer. It is not reachable: `0x77` over indices 0..511 returns only the 400
   stored slots plus block 510 (globals); `0x75` is silent. **[HW, 2026-10-29]**
2. Per-patch pitch bend range. The vendor model has only `Pitchbend target` per patch
   (`synthtool_patch_settings.csv:44`); the range is `GeneralPitchbendSemitones`, 0..24, global
   (`synthtool_global_settings.csv:18`, `gui-to-midi.csv:105`). **[CODE]** Swumpf's map has
   `pitchWheelRange` with no payload offset, which agrees. **[DATA]**
3. Sequencer, arpeggio and chord-memory pattern transfer — no frame is known.
4. Firmware updates. Separate tool (`opentribe-update`, `crates/update`).
5. Anything SysEx cannot express: audio, calibration, autotune.

## 2. Device facts the app is built on

| # | Fact | Evidence |
|---|---|---|
| F1 | Family 5, model `00 01 24`, device byte at index 7, opcode at index 8, VID:PID `1397:125F`, bootloader `1260` | `config.json`; live `lsusb` **[HW]** |
| F2 | `02` → `03 00` (user mode); `08 00` → `09 00 <maj> <min> <patch>`; this unit answers `1.4.6` | **[HW]** |
| F3 | Program read: `F0 00 20 32 00 01 24 00 77 <lsb> <msb> F7`, `index = lsb \| msb<<7`, 0..399 | **[HW]** |
| F4 | Program reply: opcode `78`, same two address bytes, then a 7-in-8 packed payload; successful reads carry 198 packed bytes → 173 unpacked | **[HW]** |
| F5 | Empty slot reply: bare `F0 F7` on this unit; the 12-byte `78 <lsb> <msb>` form appears in editor bank exports, not on the wire | **[HW, 2026-11-09]** |
| F6 | Addresses above 399 are rejected with ACK `01 00 01`, except index 510 (`7E 03`) = globals | **[HW]** |
| F7 | Patch name: unpacked offset `0x96`, NUL-terminated, 16 bytes available | **[DATA]** + Swumpf **[DATA]** |
| F8 | Program write: `78 <lsb> <msb> <198 packed bytes>` (pad trimmed records to 173 before packing); ACK `01 00 00`; read-back byte-identical for 381 writes (280 moves + 101 neutralise) and a 2026-11-09 copy/delete re-run | **[HW, 2026-10-28, 2026-11-09]** |
| F9 | Globals read: `77 7E 03` → `78 7E 03 <46 packed>` → 40 unpacked bytes | **[HW]** |
| F10 | Globals write frame not identified; `78 74 …` is what the vendor app uses at one call site | **[CODE]** — unverified |
| F11 | No per-packet checksum in the dump payload: a "last byte = sum of the rest mod 128" test matched 2 of 662 packets, i.e. chance | **[DATA]** |
| F12 | Parameter offsets exist for 52 of the 78 patch parameters, taken from the Swumpf editor's map (`swumpf_cc_map.csv`), **not verified against firmware** | **[DATA+INF]** |

Two consequences drive the whole design:

- **The PRO-800 speaks whole patches.** There is no per-parameter write frame; a change means
  read the 173-byte payload, edit bytes, write the payload back. F5 and F11 make that safe: no
  checksum to recompute, and an empty slot is unambiguous.
- **Offsets are the risk, not the transport.** Writing the wrong byte of a packed payload is a
  silent corruption, so parameter editing is gated on measured offsets (§11 G2) and ships after
  the librarian, which needs no offsets at all.

## 3. Milestones

| Milestone | Content | Gate |
|---|---|---|
| **M0 Librarian, read-only** | `list`, `dump`, `backup`, `inspect`, `diff`, JSON, MCP tools; no writes | none |
| **M1 Librarian, write** | `store`, `rename`, `copy`, `restore`, `backup --verify` on a whole patch | G1 |
| **M2 Verified parameter editing** | parameter grid for G2-passed offsets only; read-modify-write per slot | G2 |
| **M3 Globals** | read all, write G3-passed bytes; bend range first | G3 |

M0 and M1 are the deliverable that unblocks daily use; M2 must not ship with unverified offsets.

## 4. Crate layout

```text
crates/pro-800/
    Cargo.toml            name = "tribe-pro-800", bin = "opentribe-pro-800"
    src/lib.rs            re-exports, device id, GUI entry point
    src/patch.rs          7-in-8 pack/unpack, Patch, name field, address parser
    src/params.rs         Swumpf control table: sections, offsets, CCs, kinds
    src/transport.rs      blocking Session: read_slot / write_slot / globals
    src/librarian.rs      bank scan, backup, manifest, diff, restore
    src/cli.rs            headless commands
    src/app.rs            egui port: Patch, Librarian, Global, Console
    src/main.rs           thin: cli::main()
verified/pro-800-*.syx    captured programs and one global block, used by tests
```

The parameter table is hand-written from the Swumpf map because it is not firmware-verified;
it does not go through `xtask`, and every entry carries `vendor-named` except the name field
and the pitch-bend range. `xtask --check` therefore has nothing to generate for this crate.

## 5. Data model

```rust
pub struct Patch {
    pub index: u16,          // 0..=399, device address
    pub payload: Vec<u8>,    // 173 bytes unpacked, F4
    pub packed: Vec<u8>,     // 198 bytes as carried on the wire, for byte-exact re-send
    pub name: String,        // payload[0x96..], NUL-trimmed, sanitised for display only
}

pub struct Param {
    pub id: &'static str,
    pub offset: u16,         // unpacked payload offset
    pub width: Width,        // Byte | Word16Le | Bit(index)
    pub range: (u32, u32),
    pub kind: Kind,          // reuse tribe-core Kind: Byte, Bool, Enum, Percent
    pub evidence: Evidence,  // Qsg | Verified | VendorNamed | Rejected
}
```

The overlay carries `[[param]]` entries plus `[[unison_voice]]` for offsets 87..93 and
`[name] { offset = 150, len = 16 }`. Every param must state an offset **and** the fixture that
proves it; `xtask --check` fails if the fixture is missing.

Parameter inventory from the Swumpf map, to be transcribed then gated:

| block | offsets | count | note |
|---|---|---|---|
| 16-bit values (Freq A … Filt Vel) | 5,7,9,…,53 | 25 | little-endian pairs, full 16-bit after unpacking |
| `VERSION` | 4 | 1 | read-only; never written |
| flags/enums (Saw A … wheel amount) | 55..73 | 19 | one byte each |
| unison voices 1..7 | 87..93 | 7 | one byte each |
| unknown | 0..3, 74..86, 94..149 | 71 | not exposed by any editor yet |
| name | 150..165 | 16 | NUL padded |

## 6. Transport

Rules, all testable:

1. One message per slot operation; pace 30 ms between requests (backup of 400 slots ≈ 40 s, as
   `tools/alsa_sysex.py` already does).
2. Read timeout 1 s per slot; a slot that answers nothing is an error, a slot that answers with
   no payload is *empty* (F5).
3. Writes always: read the slot, mutate, re-pack, send, read back, compare. A mismatch is an
   error and the capture is kept in memory for a retry.
4. Never write a slot whose current payload is shorter/longer than 173 bytes — the layout is
   fixed and a mismatch means the read reply was malformed (the LM DRUM session saw exactly this
   failure mode).
5. Packing: 7-in-8, MSB bitmap first, identical to `packBulkDumpDataPRO800`; round-trip must be
   byte-exact for all 400 dumps in the notebook's backups.
6. Empty-slot handling: writing to an empty slot is allowed (that is how a patch is created);
   writing *over* a non-empty slot requires `--force` or an explicit slot argument.

## 7. CLI

```text
opentribe-pro-800                                 GUI
opentribe-pro-800 list-ports
opentribe-pro-800 params [--json]                 descriptor, with evidence tags
opentribe-pro-800 status [--json]                 mode, firmware version, current slot if known
opentribe-pro-800 slots [--json]                  scan 400: name, sha256, empty/init/patch
opentribe-pro-800 dump <slot> [--out FILE]        write one .syx to disk
opentribe-pro-800 backup --dir DIR [--verify]     400 dumps + manifest.csv
opentribe-pro-800 inspect --file FILE [--json]    offline: size, name, offsets, sha256
opentribe-pro-800 diff <slot|FILE> <slot|FILE>    payload and name diff
opentribe-pro-800 store --file FILE --slot S [--force]
opentribe-pro-800 restore --dir DIR [--force]
opentribe-pro-800 rename --slot S --name "…"      read-modify-write of 0x96
opentribe-pro-800 copy --from S --to T
opentribe-pro-800 get <param> --slot S
opentribe-pro-800 set <param> <value> --slot S    read-modify-write of the whole patch
opentribe-pro-800 globals [--json]                read the 40-byte block
opentribe-pro-800 globals-set <param> <value>     needs G3
opentribe-pro-800 monitor [--duration-ms N]       log unsolicited traffic
```

`set` is implemented against the Swumpf offsets and tags them `vendor-named`; gate G2 has not
been run, so the CLI does not claim the panel will show the written value. The MCP server is not
part of this crate yet.

`<slot>` accepts `B094`, `b94`, `194` and `194/0xC2`; one parser, tested.

Exit codes: `0` ok, `1` device/transport error, `2` usage or validation error, `3` verification
mismatch (write sent, read-back differs — the caller must retry or restore).

## 8. GUI

Four tabs, reusing the core shell:

| tab | content |
|---|---|
| **Patch** | parameter grid grouped OSC A/B, VCF, envelopes, LFO, polymod, glide/unison; each control labelled with its evidence tag; write goes through read-modify-write of the whole patch |
| **Librarian** | 400 slots in 4 bank columns, name/empty/Init shown, drag to copy, backup/restore buttons, disk ↔ slot compare |
| **Global** | the 40-byte block as named fields where known, hex otherwise; disabled controls for unverified offsets |
| **Console** | raw SysEx, replies, timings — same as the other devices |

The app never writes on connect. Auto-apply off by default; a pending-edit badge shows every
parameter that would be written.

## 9. Safety invariants

1. No write without an explicit user action (button, `--force`, or MCP call with `confirm`).
2. Every write is preceded by a read of the same slot; the payload is re-sent unchanged except
   for the bytes the operation owns.
3. Every write is followed by a read-back; mismatch ⇒ exit code 3 and a `.syx` capture of both
   versions in `backups/pro800-conflict-<timestamp>/`.
4. Mass operations (`backup --restore`, bank copy) require a fresh backup in the same run.
5. Name writes are limited to 16 bytes, ASCII printable, NUL padded; longer names are an error,
   not a truncation.
6. The tool refuses to touch a slot it read as malformed, and refuses to proceed if the firmware
   version is not 1.4.6 without `--allow-unknown-firmware`.

## 10. Test plan

| test | kind | pins |
|---|---|---|
| unpack/pack round trip | unit | all golden `.syx` fixtures re-pack byte-identically |
| empty slot | unit + fixture | `F0 00 20 32 00 01 24 00 78 42 01 F7` ⇒ `Patch::None` |
| name extraction | unit | C054 → `Korg Lead`, B094 → `FunkBass HB` (fixtures) |
| address encoding | unit | 0 → `00 00`, 99 → `63 00`, 194 → `42 01`, 399 → `0F 03` |
| rename | unit | read-modify-write changes only 150..165 and re-packs correctly |
| slot parser | unit | `B094`, `b94`, `194`, `194/0xC2` agree |
| refusal paths | unit | wrong payload length, name > 16, unverified param, unknown firmware |
| backup manifest | integration (offline, fixture dir) | 400 rows, sha256 stable |
| live `status` | hardware | F2 on this unit, recorded in `verified/pro-800-measurements.md` |

Fixtures: `verified/pro-800-c054.syx` (Korg Lead), `verified/pro-800-b094.syx` (FunkBass HB, the
owner's patch), `verified/pro-800-init.syx`, `verified/pro-800-globals.syx`. Each is ≤ 220 bytes.

## 11. Verification gates

**G1 — patch write path.** Passed on hardware: 381 writes, all read-back identical
(F8, notebook WORKLOG §36). Re-run 2026-11-09 on firmware 1.4.6: copy B099 → B094, read-back
identical, delete B094, read-back `F0 F7`; transcript in
`verified/pro-800-measurements.md`.

**G2 — parameter offsets.** Not passed. Procedure per parameter, using only the instrument:

1. Take a known patch, change one byte at the candidate offset to a distinctive value, write it
   to a scratch slot.
2. Select that slot on the panel and read the value in the PERF menu (the display shows the
   numeric value; pitch/filter parameters are audible too).
3. Confirm the displayed value matches the byte, restore the scratch slot from the backup.

Why this and not a dump diff: the edit buffer is unreadable (§1.1), so the only way to observe a
parameter's meaning is the panel or the sound. A read-back only proves storage, never semantics —
that trap is exactly what makes the Swumpf map **[INF]** today.

Gate: a parameter ships as `verified` only with a transcript line in
`verified/pro-800-measurements.md` naming the offset, value written, and the panel reading.

**G3 — globals write.** Not passed. Same shape: identify the frame (F10 candidate `78 74 …`),
learn which byte carries which setting by changing a value on the panel and diffing the 40-byte
block (baseline and a first diff are already saved under
`backups/pro800-preset-2026-10-29/globals-before.syx`). Pitch bend range (0..24, a fourth = 5) is
the first candidate because it is one global byte and easy to confirm by ear.

**G4 — name/rename.** Passed implicitly by F8 (a rename is a payload write), but do it once on a
scratch slot and check the panel name display.

## 12. Risks

| risk | mitigation |
|---|---|
| Swumpf offsets are wrong or shifted for some fields | they stay `vendor-named` until G2; the UI marks them and the CLI refuses `set` |
| Vendor app writes a *different* frame for globals | G3; until then `globals set` is not exposed |
| Payload size differs across firmware | refuse on length ≠ 173; log the observed length |
| Two tools writing the same unit | document that the vendor app must be closed; the CLI does not detect it |
| User expects edit-buffer capture | state it in `--help` and the README: unsaved edits cannot be read out; store on the panel first |

## 13. Open questions

1. Bytes 0..3, 74..86 and 94..149 of the payload: what are they? (Candidates: pot modes, mod
   wheel target, arp settings, chord memory, sequencer.)
2. Is the globals block really 40 bytes on every firmware, and is `PresetCurentPreset` the byte at
   index 5? The block changed from `76` to `C7` at index 5 after a program operation, which fits a
   program number, but the encoding is unconfirmed.
3. Does a `0x78` write to the *currently selected* slot update the running sound immediately, or
   only the stored copy? Unverified; the librarian does not depend on it.
4. Should the crate own a reorg/space-planning feature like `tools/pro800_reorg.py`, or stay a
   plain librarian? Proposal: stay plain; layout planning stays in the notebook tools.

## 14. References

- `devices/PRO-800/PROTOCOL-NOTES.md` — framing, dump layout, vendor call sites.
- `devices/PRO-800/MIDI-IMPLEMENTATION.md` — parameter roster, CC surface.
- `devices/PRO-800/PATCH-MAP.md` — decoded patch parameters per slot.
- `devices/PRO-800/REORG-PROPOSAL.md`, `tools/pro800_reorg*.py` — bank layout, proven move/verify
  tooling this crate replaces.
- `protocol/FIRMWARE-UPDATE.md` — why firmware is a separate tool.
- `report/WORKLOG.md` §36–§37 — the 381-write verification run.
