//! MIDI I/O. Wraps `midir` for ports and reassembles SysEx from the input
//! callback stream.

use std::sync::mpsc::{self, Receiver, Sender};

use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

const CLIENT_NAME: &str = "open-poly-d";

/// Reassembles SysEx messages. A backend is allowed to split a message across
/// callbacks, so the raw bytes are accumulated until `F7` closes the frame.
#[derive(Default)]
pub struct SysExAccum {
    buf: Vec<u8>,
}

impl SysExAccum {
    /// Feed raw port bytes; returns every SysEx message completed by this chunk.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for &byte in bytes {
            if byte == 0xF0 {
                self.buf.clear();
                self.buf.push(byte);
                continue;
            }
            if self.buf.is_empty() {
                // Not inside a SysEx frame: ignore realtime/other messages.
                continue;
            }
            self.buf.push(byte);
            if byte == 0xF7 {
                out.push(std::mem::take(&mut self.buf));
            } else if self.buf.len() > 8192 {
                // Bogus frame without an end byte; drop it.
                self.buf.clear();
            }
        }
        out
    }
}

/// Names of the currently visible output ports.
pub fn output_ports() -> Result<Vec<String>, String> {
    let midi = MidiOutput::new(CLIENT_NAME).map_err(|e| e.to_string())?;
    Ok(midi
        .ports()
        .iter()
        .filter_map(|port| midi.port_name(port).ok())
        .collect())
}

/// Names of the currently visible input ports.
pub fn input_ports() -> Result<Vec<String>, String> {
    let midi = MidiInput::new(CLIENT_NAME).map_err(|e| e.to_string())?;
    Ok(midi
        .ports()
        .iter()
        .filter_map(|port| midi.port_name(port).ok())
        .collect())
}

/// Pick the most likely POLY D port: names such as `POLY D`, `POLY D MIDI 1`,
/// `PolyD` all normalise to `polyd`.
pub fn guess_poly_d_port(names: &[String]) -> Option<String> {
    let normalise = |name: &str| -> String {
        name.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect()
    };
    names
        .iter()
        .find(|name| normalise(name).contains("polyd"))
        .cloned()
}

pub struct MidiHub {
    out: Option<MidiOutputConnection>,
    in_conn: Option<MidiInputConnection<SysExAccum>>,
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    out_name: Option<String>,
    in_name: Option<String>,
}

impl Default for MidiHub {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            out: None,
            in_conn: None,
            tx,
            rx,
            out_name: None,
            in_name: None,
        }
    }
}

impl MidiHub {
    pub fn output_name(&self) -> Option<&str> {
        self.out_name.as_deref()
    }

    pub fn input_name(&self) -> Option<&str> {
        self.in_name.as_deref()
    }

    pub fn output_open(&self) -> bool {
        self.out.is_some()
    }

    /// Open an output port by exact port name.
    pub fn connect_output(&mut self, name: &str) -> Result<(), String> {
        self.disconnect_output();
        let midi = MidiOutput::new(CLIENT_NAME).map_err(|e| e.to_string())?;
        let port = midi
            .ports()
            .into_iter()
            .find(|port| midi.port_name(port).map(|n| n == name).unwrap_or(false))
            .ok_or_else(|| format!("output port {name:?} not found"))?;
        let conn = midi
            .connect(&port, "open-poly-d-out")
            .map_err(|e| e.to_string())?;
        self.out = Some(conn);
        self.out_name = Some(name.to_string());
        Ok(())
    }

    pub fn disconnect_output(&mut self) {
        self.out = None;
        self.out_name = None;
    }

    /// Open an input port by exact port name. `notify` is called on every
    /// received chunk so the UI can repaint.
    pub fn connect_input(
        &mut self,
        name: &str,
        notify: impl Fn() + Send + 'static,
    ) -> Result<(), String> {
        self.disconnect_input();
        let midi = MidiInput::new(CLIENT_NAME).map_err(|e| e.to_string())?;
        let port = midi
            .ports()
            .into_iter()
            .find(|port| midi.port_name(port).map(|n| n == name).unwrap_or(false))
            .ok_or_else(|| format!("input port {name:?} not found"))?;
        let tx = self.tx.clone();
        let conn = midi
            .connect(
                &port,
                "open-poly-d-in",
                move |_timestamp, bytes, accum: &mut SysExAccum| {
                    for message in accum.push(bytes) {
                        let _ = tx.send(message);
                    }
                    notify();
                },
                SysExAccum::default(),
            )
            .map_err(|e| e.to_string())?;
        self.in_conn = Some(conn);
        self.in_name = Some(name.to_string());
        Ok(())
    }

    pub fn disconnect_input(&mut self) {
        self.in_conn = None;
        self.in_name = None;
    }

    /// Send one message. Errors are returned for logging, not swallowed.
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self.out.as_mut() {
            Some(conn) => conn.send(bytes).map_err(|e| e.to_string()),
            None => Err("no output port connected".to_string()),
        }
    }

    /// Drain everything the input callback has produced since the last poll.
    pub fn poll(&mut self) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        while let Ok(message) = self.rx.try_recv() {
            messages.push(message);
        }
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accum_reassembles_split_sysex() {
        let mut accum = SysExAccum::default();
        assert!(accum.push(&[0xF0, 0x00, 0x20]).is_empty());
        let messages = accum.push(&[0x32, 0x00, 0x01, 0x0C, 0x00, 0x12, 0x02, 0xF7]);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0],
            vec![
                0xF0, 0x00, 0x20, 0x32, 0x00, 0x01, 0x0C, 0x00, 0x12, 0x02, 0xF7
            ]
        );
    }

    #[test]
    fn accum_handles_back_to_back_and_junk() {
        let mut accum = SysExAccum::default();
        let messages = accum.push(&[
            0x90, 0x30, 0x7F, // stray note-on, ignored
            0xF0, 0x01, 0xF7, // first message
            0xF0, 0x02, 0xF7, // second message
        ]);
        assert_eq!(
            messages,
            vec![vec![0xF0, 0x01, 0xF7], vec![0xF0, 0x02, 0xF7]]
        );
    }

    #[test]
    fn accum_drops_unterminated_frame_before_new_start() {
        let mut accum = SysExAccum::default();
        let messages = accum.push(&[0xF0, 0x11, 0x22, 0xF0, 0x33, 0xF7]);
        assert_eq!(messages, vec![vec![0xF0, 0x33, 0xF7]]);
    }

    #[test]
    fn poly_d_port_guessing() {
        let names = vec![
            "Midi Through:Midi Through Port-0 14:0".to_string(),
            "POLY D:POLY D MIDI 1 20:0".to_string(),
        ];
        assert_eq!(
            guess_poly_d_port(&names).as_deref(),
            Some("POLY D:POLY D MIDI 1 20:0")
        );
        assert!(guess_poly_d_port(&names[..1]).is_none());
    }
}
