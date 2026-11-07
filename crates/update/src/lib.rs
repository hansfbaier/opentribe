//! Firmware updater for the Behringer / Music Tribe instruments that `opentribe` talks to.
//!
//! The updater is a *replayer*: it does not rebuild firmware messages. The vendor `.syx`
//! files are complete SysEx scripts, and the vendor application sends them byte for byte at
//! a fixed pace (see `protocol/FIRMWARE-UPDATE.md` §8.3 in the research repository). This
//! crate therefore has three jobs:
//!
//! 1. parse and describe a `.syx` stream so a wrong file is caught before anything is sent
//!    ([`firmware`]);
//! 2. identify the target instrument and produce the three control frames an update needs —
//!    mode query, reboot into DFU, firmware version ([`device`]);
//! 3. replay the stream over MIDI at the vendor pace, reporting progress and every reply
//!    ([`flash`]).
//!
//! Deliberate limits, because the unknown parts of this protocol are the dangerous ones:
//!
//! - Sending requires an explicit [`flash::Consent`]; building a [`plan::Plan`] never writes.
//! - The DFU-mode MIDI port is unverified on Linux. If the device does not reappear, the
//!   updater says so and stops; it does not guess another port.
//! - Neutron is listed but not driven: its firmware is an STM32 USB DFU image, not SysEx.
//!
//! ```text
//! opentribe-update devices                     list instruments and their firmware flavour
//! opentribe-update inspect --file fw.syx       describe a firmware stream, no MIDI
//! opentribe-update ports                       MIDI ports currently visible
//! opentribe-update status --device pro-800     mode and firmware version, read-only
//! opentribe-update flash --device pro-800 --file fw.syx [--send]
//! ```

pub mod device;
pub mod firmware;
pub mod flash;
pub mod md5;
pub mod plan;

pub use device::{Device, Flavour, DEVICES};
pub use firmware::{Firmware, Segment, Shape, Summary};
pub use flash::{Consent, ReplayReport, Sink};
pub use plan::Plan;

/// Pace the vendor application uses between two `.syx` messages.
pub const VENDOR_PACE: std::time::Duration = std::time::Duration::from_millis(10);
/// How long the vendor application waits after `03 30` before reconnecting.
pub const VENDOR_RECONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(8);
