# LM DRUM — measurement transcript

Instrument: Behringer LM DRUM, USB `1397:1267`, model bytes `00 01 28`, device ID `00`,
firmware **1.1.7**, protocol family 3. Port: ALSA rawmidi (`hw:1,0,0`).

Everything below was produced by sending bytes to the instrument and reading the reply. The
public manual has no SysEx packet table for this device, so nothing here relies on one.

## 1. Identification

| Request | Reply | Reading |
| --- | --- | --- |
| `F0 00 20 32 00 01 28 00 08 00 F7` | `F0 00 20 32 00 01 28 00 09 00 01 01 07 F7` | opcode `09` = firmware, payload `00 01 01 07` → 1.1.7 |
| `F0 00 20 32 00 01 28 00 02 F7` | `F0 00 20 32 00 01 28 00 03 00 F7` | operation mode normal |

## 2. Settings read

| Request | Reply |
| --- | --- |
| `F0 00 20 32 00 01 28 00 75 F7` | `F0 00 20 32 00 01 28 00 76 …108 bytes… F7` |

Baseline block (offsets 0..107):

```text
00 00 01 00 00 00 67 10 00 00 00 10 00 00 24 28 29 26 27 32 2E 2F 33 31
2A 2B 2C 2D 30 25 02 00 0A 0B 0E 00 6D 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01
00 00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 00 01 0B 06 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 02 02 02 02 02 01 01 02 02 02 02 01 01 01 01 02
```

## 3. ACK behaviour

Accepted-looking writes came back either as `01 00 00` or `01 00 01`, and the block changed in
both cases, so the trailing code is **not** a success flag on this device. `0x77 7E 03` also
returns `01 00 01`; the POLY D's "rejected = 5" rule does not transfer.

## 4. Writes and block diffs

Method: read block, write one packet, read block again, diff against the baseline, then
restore. Only packets whose effect is visible in the block are included.

```text
channel 0E 01 03 04   ack 01 00 01   offsets 8:=03, 7:=04
channel 0E 01 05 0A   ack 01 00 01   offsets 8:=05, 7:=0A      (payload order confirmed)
channel 0E 01 00 10   ack 01 00 01   offsets 7:=10, 8:=00      (restore)
note map 0x29 +16     ack 01 00 00   offsets 14..29 := 30..45
note map restore      ack 01 00 00   offsets 14..29 := baseline
0x2A 02               ack 01 00 00   offset 30 := 02
0x1C 6D               ack 01 00 00   offset 36 := 109
0x1B 00               ack 01 00 00   offset 5 := 00
0x17 / 0x19 / 0x1A / 0x26 / 0x1D / 0x1F: no block change observed
```

Resulting layout for the descriptors:

| offset | parameter | packet |
| --- | --- | --- |
| 7 | MIDI channel RX | `0E 01 <tx> <rx>` |
| 8 | MIDI channel TX | `0E 01 <tx> <rx>` |
| 14-29 | note map, 16 slots | `29` |
| 30 | 0x2A value | `2A` |
| 36 | 0x1C value | `1C` |
| 6 | clock rate | `1A` |
| 54 | sample mode | `74 0A` |
| 5 | 0x1B value | `1B` |

The final block equals the baseline byte for byte.

## 5. Documented CCs

From the LM DRUM user manual p. 27: CC 74 filter cutoff; CC 75-90 voice tunings (bass drum,
snare, sidestick, closed hi-hat, open hi-hat, high tom, mid tom, low tom, ride, crash, cabasa,
tambourine, high conga, low conga, cowbell, claps). These are ordinary channel-voice messages,
so they need no protocol measurement; the app sends them on the RX channel read from the block.

## 6. Open questions

- The voice behind each note-map slot is not published for packet `0x29`; slots are named by
  position and default to the values the instrument reported.
- Packet `0x1C` changes offset 36 but its purpose is not documented; the app exposes it as a
  neutral byte with a range.
- The sub-packet settings (`0x74` with a sub-index) and the sequencer/sample paths are not
  covered; they need their own investigation before they can be added with a source.
