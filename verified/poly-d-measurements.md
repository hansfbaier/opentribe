# POLY D — measurement transcript

Instrument: Behringer POLY D, USB `1397:122a`, device ID `00`, model bytes `00 01 0C`,
firmware **1.1.3**, protocol family 3. Port: ALSA rawmidi (`hw:0,0,0`).

Everything below was produced by sending bytes to the instrument and reading the reply.
Firmware was queried from the device itself; no vendor software or data file is involved.
Byte strings are the complete SysEx message unless noted.

## 1. Identification

| Request | Reply | Reading |
| --- | --- | --- |
| `F0 00 20 32 00 01 0C 00 08 00 F7` | `F0 00 20 32 00 01 0C 00 09 00 01 01 03 F7` | opcode `09` = firmware, payload `00 01 01 03` → 1.1.3 |
| `F0 00 20 32 00 01 0C 00 02 F7` | `F0 00 20 32 00 01 0C 00 03 00 F7` | opcode `03` = operation mode, `00` = normal |

## 2. Settings read

| Request | Reply |
| --- | --- |
| `F0 00 20 32 00 01 0C 00 75 F7` | `F0 00 20 32 00 01 0C 00 76 …25 bytes… F7` |

Baseline block (offsets 0..24):

```text
00 00 00 0C 00 00 00 00 01 05 02 00 24 02 00 00 01 60 03 03 03 03 03 03 03
```

## 3. Rejected requests

| Request | Reply | Conclusion |
| --- | --- | --- |
| `… 00 77 7E 03 F7` | `… 00 01 00 05 F7` | long read `0x77` rejected |
| `… 00 77 74 03 F7` | `… 00 01 00 05 F7` | long read `0x77` rejected |
| `… 00 78 11 0C 00 F7` | `… 00 01 00 05 F7` | `0x78` long-envelope framing rejected |
| `… 00 78 2F 01 F7` | `… 00 01 00 05 F7` | `0x78` framing rejected |
| `… 00 12 05 F7` | `… 00 01 00 05 F7` | key priority 5 out of range |
| `… 00 2F 02 F7` | `… 00 01 00 05 F7` | local keyboard 2 out of range |
| `… 00 30 00 F7` | `… 00 01 00 05 F7` | unknown packet `0x30` |
| `… 00 1D 01 F7` | `… 00 01 00 05 F7` | sequencer auto-sync not supported |
| `… 00 1F 42 F7` | `… 00 01 00 05 F7` | assign value not supported |

ACK payload `01 00 00` means accepted; `01 00 05` means rejected. Every accepted write below
returned `01 00 00`.

## 4. Packet framing

Accepted form (QSG):

```text
F0 00 20 32 00 01 0C 00 0E 01 03 04 F7     channel TX 3 / RX 4
```

The `0x78`-prefixed form is rejected, so the app defaults to the QSG form and keeps the other
one only as a selectable experiment.

## 5. Read-back offset mapping

Method: read the block, write one packet with a value that differs from the block, read the
block again, diff. Each write was followed by a restore write; the final block equals the
baseline.

```
baseline block: 00 00 00 0C 00 00 00 00 01 05 02 00 24 02 00 00 01 60 03 03 03 03 03 03 03

transpose          wrote 0F 05              ack 01 00 00  changed=[3]        ok
velocity           wrote 10 11 22 01        ack 01 00 00  changed=[4, 5, 6]  ok
key_priority       wrote 12 02              ack 01 00 00  changed=[7]        ok
multi_trigger      wrote 14 00 00           ack 01 00 00  changed=[8]        ok
pitch_bend         wrote 11 11 00           ack 01 00 00  changed=[9]        ok
mod_curve          wrote 15 01              ack 01 00 00  changed=[11]       ok
note_zero_cv       wrote 16 30              ack 01 00 00  changed=[12]       ok
clock_rate         wrote 1A 03              ack 01 00 00  changed=[13]       ok
clock_source       wrote 1B 01              ack 01 00 00  changed=[14]       ok
local_keyboard     wrote 2F 01              ack 01 00 00  changed=[15]       ok
clock_polarity     wrote 19 00              ack 01 00 00  changed=[16]       ok
accent_threshold   wrote 1C 40              ack 01 00 00  changed=[17]       ok
clock_out          wrote 17 01              ack 01 00 00  changed=[18]       ok
out_pitch_wheel    wrote 22 02              ack 01 00 00  changed=[19]       ok
out_mod_wheel      wrote 21 01              ack 01 00 00  changed=[20]       ok
out_keyboard       wrote 23 00              ack 01 00 00  changed=[21]       ok
out_aftertouch     wrote 24 02              ack 01 00 00  changed=[22]       ok
out_sequencer      wrote 25 01              ack 01 00 00  changed=[23]       ok
out_arp            wrote 26 01              ack 01 00 00  changed=[24]       ok
mod_wheel_range    wrote 20 04              ack 01 00 00  changed=[10]       ok
channel            wrote 0E 01 03 09        ack 01 00 00  changed=[1, 2]     ok

final block:    00 00 00 0C 00 00 00 00 01 05 02 00 24 02 00 00 01 60 03 03 03 03 03 03 03
restored:       yes
```

Resulting layout:

| offset | parameter | packet |
| --- | --- | --- |
| 1 | MIDI channel RX | `0E 01 <tx> <rx>` |
| 2 | MIDI channel TX | `0E 01 <tx> <rx>` |
| 3 | MIDI IN transpose | `0F` |
| 4 | note-on velocity | `10` |
| 5 | note-off velocity | `10` |
| 6 | velocity curve | `10` |
| 7 | key priority | `12` |
| 8 | multi-trigger | `14` |
| 9 | pitch bend range | `11` |
| 10 | modulation wheel range | `20` |
| 11 | modulation curve | `15` |
| 12 | note at zero CV | `16` |
| 13 | clock rate | `1A` |
| 14 | clock source | `1B` |
| 15 | local keyboard mode | `2F` |
| 16 | clock polarity | `19` |
| 17 | accent velocity threshold | `1C` |
| 18 | MIDI clock out | `17` |
| 19 | pitch wheel output | `22` |
| 20 | mod wheel output | `21` |
| 21 | keyboard output | `23` |
| 22 | keyboard aftertouch output | `24` |
| 23 | sequencer output | `25` |
| 24 | arpeggiator output | `26` |

Offsets 0, 5 and 6: offset 0 read as zero in every block seen and no write changed it; 5 and 6
are confirmed above (velocity). None of the three is exposed as a parameter.

## 6. Notes

- The block is 25 bytes; the app rejects shorter replies instead of partially applying them.
- Writes are not persisted for local keyboard mode: it read `0` after `2F 00`, then `1` after a
  power cycle, without any other write in between.
- Repeatedly opening and closing the host MIDI port eventually stops the instrument replying
  to SysEx. Power-cycling restores it. This was observed with raw probes as well, and on an
  LM DRUM that the app never opened.
