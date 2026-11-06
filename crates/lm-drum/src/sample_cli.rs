//! `opentribe-lm-drum-sample` — upload one WAV to the LM DRUM sample pool.
//!
//! The audio must already be mono, 24 kHz, 16-bit PCM. Conversion is
//! deliberately left to ffmpeg/sox so this tool stays dependency-free and
//! clean-room:
//!
//! ```text
//! ffmpeg -i in.wav -ac 1 -ar 24000 -sample_fmt s16 out.wav
//! opentribe-lm-drum-sample out.wav --name MY-SAMPLE          # dry run
//! opentribe-lm-drum-sample out.wav --name MY-SAMPLE --send
//! ```

use std::time::{Duration, Instant};

use tribe_core::midi::{self, MidiHub};
use tribe_lm_drum::sample::{self, CHUNK_BYTES, SAMPLE_RATE, SAMPLE_WIDTH};
use tribe_lm_drum::DEVICE;

const ACK_TIMEOUT: Duration = Duration::from_secs(5);

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("opentribe-lm-drum-sample: {err}");
            std::process::ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut file = None;
    let mut name = None;
    let mut port = None;
    let mut chunk = CHUNK_BYTES;
    let mut send = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = Some(args.next().ok_or("--name needs a value")?),
            "--port" => port = Some(args.next().ok_or("--port needs a value")?),
            "--chunk" => {
                chunk = args
                    .next()
                    .ok_or("--chunk needs a value")?
                    .parse::<usize>()
                    .map_err(|_| "--chunk must be a number")?
            }
            "--send" => send = true,
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other if other.starts_with('-') => return Err(format!("unknown flag {other}")),
            other => {
                if file.replace(other.to_string()).is_some() {
                    return Err("only one WAV file may be given".into());
                }
            }
        }
    }
    let file = file.ok_or("a WAV file is required")?;
    let bytes = std::fs::read(&file).map_err(|e| format!("{file}: {e}"))?;
    let pcm = pcm_from_wav(&bytes)?;
    let name = name.unwrap_or_else(|| {
        std::path::Path::new(&file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("SAMPLE")
            .to_string()
    });

    let frames = sample::frames(&pcm, &name, SAMPLE_RATE, SAMPLE_WIDTH, chunk)?;
    println!("name        {name}");
    println!("pcm         {} bytes (mono, {SAMPLE_RATE} Hz, s16le)", pcm.len());
    println!("packets     {} ({} data)", frames.len(), frames.len().saturating_sub(1));
    println!("chunk       {chunk} bytes");
    println!("header      {}", hex(&frames[0]));
    if !send {
        println!("dry run     nothing sent; add --send to transmit");
        return Ok(());
    }

    let names = midi::output_ports()?;
    let port_name = match port {
        Some(exact) => names
            .iter()
            .find(|n| *n == &exact)
            .cloned()
            .or_else(|| midi::guess_port(&names, &exact))
            .ok_or_else(|| format!("no MIDI output matching {exact:?}"))?,
        None => midi::guess_port(&names, DEVICE.port_hint)
            .ok_or_else(|| format!("no MIDI output for {}: {names:?}", DEVICE.name))?,
    };
    let mut hub = MidiHub::default();
    hub.connect_output(&port_name)?;
    hub.connect_input(&port_name, || {})?;
    println!("port        {port_name}");
    for (index, frame) in frames.iter().enumerate() {
        hub.send(frame)?;
        let status = wait_ack(&mut hub)?;
        if status != 0 {
            return Err(format!("packet {index} rejected with ACK status {status}"));
        }
        println!("  {index:3} {:5} bytes  ACK 00", frame.len());
    }
    println!("uploaded    sample is in the global pool; assign it to a pad separately");
    Ok(())
}

fn wait_ack(hub: &mut MidiHub) -> Result<u8, String> {
    let deadline = Instant::now() + ACK_TIMEOUT;
    while Instant::now() < deadline {
        for message in hub.poll() {
            if message.len() >= 11 && message[8] == 0x01 && message[10] == 0x00 {
                return Ok(0);
            }
            if message.len() >= 11 && message[8] == 0x01 {
                return Ok(message[10]);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err("no ACK from the instrument".into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

fn pcm_from_wav(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut pos = 12usize;
    let mut format = None;
    let mut data = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        let end = pos.checked_add(size).ok_or("WAV chunk size overflow")?;
        if end > bytes.len() {
            return Err("truncated WAV chunk".into());
        }
        match id {
            b"fmt " => {
                if size < 16 {
                    return Err("short fmt chunk".into());
                }
                let mut tag = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
                let channels = u16::from_le_bytes([bytes[pos + 2], bytes[pos + 3]]);
                let rate = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
                let bits = u16::from_le_bytes([bytes[pos + 14], bytes[pos + 15]]);
                if tag == 0xFFFE && size >= 26 {
                    tag = u16::from_le_bytes([bytes[pos + 24], bytes[pos + 25]]);
                }
                format = Some((tag, channels, rate, bits));
            }
            b"data" => data = Some(bytes[pos..end].to_vec()),
            _ => {}
        }
        pos = end + (size & 1);
    }
    let (tag, channels, rate, bits) = format.ok_or("WAV has no fmt chunk")?;
    if tag != 1 {
        return Err(format!("WAV format {tag} is not PCM; convert to 16-bit PCM"));
    }
    if channels != 1 || rate != SAMPLE_RATE || bits != SAMPLE_WIDTH {
        return Err(format!(
            "WAV is {channels} ch, {rate} Hz, {bits}-bit; convert first: \
             ffmpeg -i IN -ac 1 -ar {SAMPLE_RATE} -sample_fmt s16 OUT.wav"
        ));
    }
    let data = data.ok_or("WAV has no data chunk")?;
    if data.len() % 2 != 0 {
        return Err("WAV data length is not a multiple of 2".into());
    }
    Ok(data)
}

fn print_help() {
    println!(
        "opentribe-lm-drum-sample — upload one WAV to the LM DRUM sample pool\n\
         \n\
         usage: opentribe-lm-drum-sample FILE.wav [--name NAME] [--port NAME] [--send]\n\
         \n\
         FILE must be mono 24 kHz 16-bit PCM. Convert with:\n\
           ffmpeg -i in.wav -ac 1 -ar 24000 -sample_fmt s16 out.wav\n\
         \n\
         Without --send the frames are printed as a plan. The sample is appended\n\
         to the global pool; assign it to a pad afterwards."
    );
}
