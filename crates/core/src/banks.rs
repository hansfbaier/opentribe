//! Bank-record import/export.
//!
//! The vendor exports one bank as a single SysEx frame:
//! `F0 00 20 32 <model> <DID> 78 <40+bank> 00 00 <packed> F7`.
//! The unpacked payload is the bank record (497 bytes for most LM DRUM banks).
//! Importing is a direct write of that frame; the device ACKs and the app reads
//! the bank back to verify.

use std::path::{Path, PathBuf};

use crate::descriptor::{BankSpec, Device};
use crate::sysex;

/// A bank frame loaded from a `.syx` file.
#[derive(Clone, Debug)]
pub struct BankFile {
    pub path: PathBuf,
    /// Bank encoded in the file's sub-opcode.
    pub source_bank: u8,
    /// Unpacked bank record.
    pub payload: Vec<u8>,
}

/// Bank protocol for a device, if it has one.
pub fn spec(device: &Device) -> Option<BankSpec> {
    device.samples?.banks
}

/// Sub-opcode for a bank.
pub fn sub(spec: BankSpec, bank: u8) -> Option<u8> {
    if usize::from(bank) >= spec.count {
        return None;
    }
    spec.sub_base.checked_add(bank)
}

/// Read one bank (`77 40+bank`).
pub fn read_request(device: &Device, device_id: u8, bank: u8) -> Option<Vec<u8>> {
    let spec = spec(device)?;
    let sub = sub(spec, bank)?;
    Some(sysex::op_message(
        device.model,
        device.family,
        device_id,
        spec.read_opcode,
        &[sub],
    ))
}

/// Parse a bank reply body (`78 <sub> 00 00 <packed>`).
pub fn parse_reply(device: &Device, body: &[u8]) -> Option<(u8, Vec<u8>)> {
    let spec = spec(device)?;
    let sub = *body.get(1)?;
    let bank = sub.checked_sub(spec.sub_base)?;
    if usize::from(bank) >= spec.count {
        return None;
    }
    let packed = body.strip_prefix(&[spec.write_opcode, sub, 0x00, 0x00])?;
    let payload = sysex::unpack_7in8(packed);
    if payload.is_empty() {
        return None;
    }
    Some((bank, payload))
}

/// Build a write frame for one bank, overriding the bank in the payload.
pub fn write_frame(
    device: &Device,
    device_id: u8,
    bank: u8,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    let spec = spec(device).ok_or("this device has no bank records")?;
    let sub = sub(spec, bank).ok_or_else(|| format!("bank {bank} is outside 0..{}", spec.count))?;
    if payload.is_empty() {
        return Err("bank record is empty".to_string());
    }
    let mut body = vec![spec.write_opcode, sub, 0x00, 0x00];
    body.extend(sysex::pack_7in8(payload));
    Ok(sysex::envelope(
        device.model,
        device.family,
        device_id,
        &body,
    ))
}

/// Load a vendor bank export. The file must contain exactly one bank frame.
pub fn load_file(device: &Device, path: &Path) -> Result<BankFile, String> {
    let spec = spec(device).ok_or("this device has no bank records")?;
    let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let message = sysex::parse(device.model, device.family, &bytes)
        .ok_or_else(|| format!("{}: not a {} SysEx frame", path.display(), device.name))?;
    let body = message.body;
    let sub = *body.get(1).ok_or("bank frame is too short")?;
    let bank = sub
        .checked_sub(spec.sub_base)
        .filter(|bank| usize::from(*bank) < spec.count)
        .ok_or_else(|| format!("{}: sub-opcode {sub:02X} is not a bank", path.display()))?;
    let packed = body
        .strip_prefix(&[spec.write_opcode, sub, 0x00, 0x00])
        .ok_or("bank frame does not start with the write opcode")?;
    let payload = sysex::unpack_7in8(packed);
    if payload.is_empty() {
        return Err(format!("{}: bank record is empty", path.display()));
    }
    Ok(BankFile {
        path: path.to_path_buf(),
        source_bank: bank,
        payload,
    })
}

/// Save a bank payload as a vendor-compatible `.syx` frame.
pub fn save_file(
    device: &Device,
    device_id: u8,
    bank: u8,
    payload: &[u8],
    path: &Path,
) -> Result<(), String> {
    let frame = write_frame(device, device_id, bank, payload)?;
    std::fs::write(path, frame).map_err(|err| format!("{}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trip_preserves_high_bits() {
        let data: Vec<u8> = (0..=251).collect();
        assert_eq!(sysex::unpack_7in8(&sysex::pack_7in8(&data)), data);
    }
}
