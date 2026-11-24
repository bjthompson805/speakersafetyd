// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) The Asahi Linux Contributors
/*!
    Platform I/O layer. Everything above this module (thermal model, group
    linking, heartbeat, panic-on-error) is platform independent; each
    backend provides the same set of types with the same methods:

    - `Card::open(maker, model)`: open the speaker card and take the
      element lock on its controls for the life of the returned value.
    - `Elem::new(id, &card)`: one control. `read_int` / `write_int`,
      `read_bool` / `write_bool`, `db_range` (mB), `from_db` (mB to raw,
      alsa-lib `Round::Floor` semantics) and `to_db` (raw to mB). Every
      error panics, as upstream does.
    - `SensePcm::open(&card, &globals)` and `readi(&mut [i16])`: blocking
      interleaved S16 reads of whole frames.

    Linux: ALSA (alsa-rs). FreeBSD: sysctl(3) + /dev/spkctlN + OSS
    (docs/freebsd.md).
*/

/// Which control of the speaker card an `Elem` refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtlKind {
    VSense,
    ISense,
    AmpGain,
    Volume,
    SampleRate,
    Unlock,
}

/// A control: the ALSA element name (Linux) and the speaker it belongs to
/// plus its kind (FreeBSD, which names nodes per amp).
#[derive(Clone, Debug)]
pub struct CtlId {
    pub alsa_name: String,
    pub speaker: Option<String>,
    pub kind: CtlKind,
}

impl CtlId {
    pub fn card(alsa_name: &str, kind: CtlKind) -> Self {
        CtlId {
            alsa_name: alsa_name.to_string(),
            speaker: None,
            kind,
        }
    }

    pub fn amp(speaker: &str, alsa_name: String, kind: CtlKind) -> Self {
        CtlId {
            alsa_name,
            speaker: Some(speaker.to_string()),
            kind,
        }
    }
}

/// Why a sense read returned no frames.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ReadError {
    /// System suspend (Linux ESTRPIPE): reopen the PCM.
    Suspended,
    /// No data within the driver's timeout (FreeBSD EAGAIN while the
    /// speaker stream is stopped): retry.
    NoData,
    /// Anything else (errno).
    Other(i32),
}

#[cfg(target_os = "linux")]
mod alsa;
#[cfg(target_os = "linux")]
pub use self::alsa::*;

#[cfg(target_os = "freebsd")]
mod freebsd;
#[cfg(target_os = "freebsd")]
pub use self::freebsd::*;
