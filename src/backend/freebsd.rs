// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) The Asahi Linux Contributors
/*!
    FreeBSD backend: the M5 speaker interface of apple_macaudio
    (m1n1-env docs/speaker-interface.md).

    - Controls are sysctls under dev.pcm.<spk>.speaker. Holding
      /dev/spkctl<spk> open is the element lock: the kernel lets only the
      pid holding it write the owner controls, so all writes go through
      sysctlbyname(3) from this process.
    - The sense PCM is OSS on /dev/dsp<sense>: S16_LE, 12 channels,
      bit-perfect, blocking reads that return EAGAIN after hw.snd.timeout
      seconds without data.
*/

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use log::{info, warn};

use super::{CtlId, CtlKind, ReadError};
use crate::types::Globals;

/// Highest pcm unit probed during discovery.
const MAX_PCM_UNITS: u32 = 32;
/// device_set_desc() of the sense pcm in apple_macaudio_spk.c.
const SENSE_DESC: &str = "Apple M5 audio (speaker sense)";
/// The only rate the speaker link runs at (MAS_RATE).
const SENSE_RATE: i32 = 48000;

/* <sys/soundcard.h>: _IOWR('P', n, int) */
const SNDCTL_DSP_SPEED: libc::c_ulong = 0xc004_5002;
const SNDCTL_DSP_SETFMT: libc::c_ulong = 0xc004_5005;
const SNDCTL_DSP_CHANNELS: libc::c_ulong = 0xc004_5006;
const SNDCTL_DSP_SETFRAGMENT: libc::c_ulong = 0xc004_500a;
const AFMT_S16_LE: libc::c_int = 0x10;

/// TLV mute value alsa-lib reports as a dB range minimum.
const SND_CTL_TLV_DB_GAIN_MUTE: i64 = -9999999;

#[cfg(not(target_endian = "little"))]
compile_error!("the sense PCM is S16_LE and is read in place as native i16");

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn errstr(e: i32) -> String {
    io::Error::from_raw_os_error(e).to_string()
}

pub fn sysctl_get_int(name: &str) -> Result<i32, i32> {
    let cname = CString::new(name).unwrap();
    let mut v: libc::c_int = 0;
    let mut len: libc::size_t = std::mem::size_of::<libc::c_int>();
    let r = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            &mut v as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if r != 0 {
        return Err(errno());
    }
    if len != std::mem::size_of::<libc::c_int>() {
        panic!("{}: not an int sysctl (len {})", name, len);
    }
    Ok(v)
}

pub fn sysctl_set_int(name: &str, v: i32) -> Result<(), i32> {
    let cname = CString::new(name).unwrap();
    let v: libc::c_int = v;
    let r = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &v as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>(),
        )
    };
    if r != 0 {
        Err(errno())
    } else {
        Ok(())
    }
}

pub fn sysctl_get_string(name: &str) -> Result<String, i32> {
    let cname = CString::new(name).unwrap();
    let mut len: libc::size_t = 0;
    let r = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if r != 0 {
        return Err(errno());
    }
    let mut buf = vec![0u8; len + 1];
    let mut len2 = buf.len();
    let r = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len2,
            std::ptr::null(),
            0,
        )
    };
    if r != 0 {
        return Err(errno());
    }
    buf.truncate(len2);
    while buf.last() == Some(&0) {
        buf.pop();
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn get_int(name: &str) -> i32 {
    sysctl_get_int(name).unwrap_or_else(|e| panic!("Could not read {}: {}", name, errstr(e)))
}

/// First entry of the root node's "compatible" (hw.fdt.compatible joins
/// the list with spaces), e.g. "apple,j815".
pub fn fdt_compatible() -> String {
    let compat = sysctl_get_string("hw.fdt.compatible")
        .unwrap_or_else(|e| panic!("Could not read hw.fdt.compatible: {}", errstr(e)));
    compat
        .split(|c: char| c == ' ' || c == '\0')
        .find(|s| !s.is_empty())
        .expect("Empty hw.fdt.compatible")
        .to_string()
}

/// The sysctl node name the kernel gives an amp: its sound-name-prefix
/// lower-cased, non-alphanumerics as '_'.
pub fn amp_node(speaker: &str) -> String {
    speaker
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub struct Card {
    /// /dev/spkctl<spk_unit>: held for the life of the daemon.
    _owner: File,
    spk_unit: u32,
    sense_unit: u32,
}

impl Card {
    fn root(&self) -> String {
        format!("dev.pcm.{}.speaker", self.spk_unit)
    }

    pub fn open(maker: &str, model: &str) -> Card {
        info!("Device: {},{} (FreeBSD apple_macaudio)", maker, model);
        let mut spk_unit = None;
        let mut sense_unit = None;
        for n in 0..MAX_PCM_UNITS {
            if sysctl_get_int(&format!("dev.pcm.{}.speaker.amps", n)).is_ok() {
                if spk_unit.is_some() {
                    panic!("More than one speaker pcm (dev.pcm.{}.speaker)", n);
                }
                spk_unit = Some(n);
            }
            if let Ok(desc) = sysctl_get_string(&format!("dev.pcm.{}.%desc", n)) {
                if desc == SENSE_DESC {
                    if sense_unit.is_some() {
                        panic!("More than one speaker sense pcm (pcm{})", n);
                    }
                    sense_unit = Some(n);
                }
            }
        }
        let spk_unit = spk_unit.expect("No speaker pcm (dev.pcm.N.speaker): apple_macaudio not loaded?");
        let sense_unit = sense_unit.expect("No speaker sense pcm: no protection possible");
        info!("Speaker pcm: pcm{}, sense pcm: pcm{}", spk_unit, sense_unit);

        let path = format!("/dev/spkctl{}", spk_unit);
        let owner = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                let holder = sysctl_get_int(&format!("dev.pcm.{}.speaker.owner_pid", spk_unit));
                panic!("{}: Could not open owner device: {} (owner_pid {:?})", path, e, holder);
            }
        };

        let card = Card {
            _owner: owner,
            spk_unit,
            sense_unit,
        };
        let pid = unsafe { libc::getpid() };
        let owner_pid = get_int(&format!("{}.owner_pid", card.root()));
        assert_eq!(owner_pid, pid, "{}: owner is not this process", path);
        let magic = get_int(&format!("{}.unlock_magic", card.root()));
        assert_eq!(magic, crate::UNLOCK_MAGIC, "Kernel unlock magic differs");
        let failed = get_int(&format!("{}.failed", card.root()));
        if failed != 0 {
            panic!("Kernel speaker lock failed (failed = {}); reload apple_macaudio", failed);
        }
        let iface = get_int(&format!("{}.interface", card.root()));
        if iface != 0 {
            // EBUSY on every ping until root switches it back; fail loudly now.
            panic!("{}.interface is {} (lease); speakersafetyd needs 0", card.root(), iface);
        }
        info!("Holding {} (element lock)", path);
        card
    }
}

/// One sysctl control. `max_node` is re-read on every conversion because
/// volume_max changes with the lock.
pub struct Elem {
    elem_name: String,
    node: String,
    min_node: Option<String>,
    max_node: Option<String>,
    db_min: i64,
    db_step: i64,
    mute_at_min: bool,
}

impl Elem {
    pub fn new(cid: CtlId, card: &Card) -> Elem {
        let root = card.root();
        let base = match &cid.speaker {
            Some(spk) => {
                let base = format!("{}.{}", root, amp_node(spk));
                let kname = sysctl_get_string(&format!("{}.name", base))
                    .unwrap_or_else(|e| panic!("{}: no amp node {}: {}", spk, base, errstr(e)));
                assert_eq!(&kname, spk, "{}: kernel amp name differs", base);
                base
            }
            None => root,
        };
        let (leaf, range) = match cid.kind {
            CtlKind::VSense => ("vsense", None),
            CtlKind::ISense => ("isense", None),
            CtlKind::AmpGain => ("amp_gain", Some(("amp_gain", false))),
            CtlKind::Volume => ("volume", Some(("volume", true))),
            CtlKind::SampleRate => ("sample_rate", None),
            CtlKind::Unlock => ("unlock", None),
        };
        let mut e = Elem {
            elem_name: cid.alsa_name,
            node: format!("{}.{}", base, leaf),
            min_node: None,
            max_node: None,
            db_min: 0,
            db_step: 0,
            mute_at_min: false,
        };
        if let Some((p, mute)) = range {
            e.min_node = Some(format!("{}.{}_min", base, p));
            e.max_node = Some(format!("{}.{}_max", base, p));
            e.db_min = get_int(&format!("{}.{}_db_min", base, p)) as i64;
            e.db_step = get_int(&format!("{}.{}_db_step", base, p)) as i64;
            assert!(e.db_step > 0, "{}: bad dB step", e.node);
            e.mute_at_min = mute;
        }
        // Same as the ALSA path: fail at setup if the control is unreadable.
        get_int(&e.node);
        e
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        &self.elem_name
    }

    pub fn read_int(&mut self, _card: &Card) -> i32 {
        sysctl_get_int(&self.node).unwrap_or_else(|e| {
            panic!("Could not read elem value {} ({}): {}", self.elem_name, self.node, errstr(e))
        })
    }

    pub fn write_int(&mut self, _card: &Card, value: i32) {
        if let Err(e) = sysctl_set_int(&self.node, value) {
            panic!(
                "Could not write elem value {} ({} = {}): {}",
                self.elem_name, self.node, value, errstr(e)
            );
        }
    }

    pub fn read_bool(&mut self, card: &Card) -> bool {
        match self.read_int(card) {
            0 => false,
            1 => true,
            v => panic!("{}: not a switch value: {}", self.node, v),
        }
    }

    pub fn write_bool(&mut self, card: &Card, value: bool) {
        self.write_int(card, value as i32);
    }

    fn raw_range(&self) -> (i64, i64) {
        let min = get_int(self.min_node.as_ref().expect("no range")) as i64;
        let max = get_int(self.max_node.as_ref().expect("no range")) as i64;
        assert!(min <= max, "{}: empty range", self.node);
        (min, max)
    }

    /// (min, max) in mB, as snd_ctl_get_dB_range().
    pub fn db_range(&mut self, _card: &Card) -> (i64, i64) {
        let (min, max) = self.raw_range();
        let lo = if self.mute_at_min && min == 0 {
            SND_CTL_TLV_DB_GAIN_MUTE
        } else {
            self.db_min + min * self.db_step
        };
        (lo, self.db_min + max * self.db_step)
    }

    /// snd_ctl_convert_from_dB(..., Round::Floor), clamped to the current
    /// [min, max].
    pub fn from_db(&mut self, _card: &Card, mb: i64) -> i32 {
        let (min, max) = self.raw_range();
        let v = (mb - self.db_min).div_euclid(self.db_step);
        v.clamp(min, max) as i32
    }

    /// Raw value to mB (value 0 of a muting control: the mute value).
    pub fn to_db(&self, _card: &Card, val: i32) -> i64 {
        if self.mute_at_min && val == 0 {
            return SND_CTL_TLV_DB_GAIN_MUTE;
        }
        self.db_min + val as i64 * self.db_step
    }
}

pub struct SensePcm {
    dsp: File,
    frame_bytes: usize,
}

impl SensePcm {
    pub fn open(card: &Card, globals: &Globals) -> SensePcm {
        if globals.visense_pcm as u32 != card.sense_unit {
            info!(
                "Sense pcm is pcm{} (conf visense_pcm = {} is the ALSA device number)",
                card.sense_unit, globals.visense_pcm
            );
        }
        let path = format!("/dev/dsp{}", card.sense_unit);
        let dsp = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(&path)
            .unwrap_or_else(|e| panic!("{}: Could not open sense PCM: {}", path, e));
        let fd = dsp.as_raw_fd();
        let frame_bytes = globals.channels * 2;

        let ioctl = |req: libc::c_ulong, val: libc::c_int, what: &str| -> libc::c_int {
            let mut v = val;
            if unsafe { libc::ioctl(fd, req, &mut v) } != 0 {
                panic!("{}: {} failed: {}", path, what, io::Error::last_os_error());
            }
            v
        };

        // Fragments: the largest power of two <= one period, and enough of
        // them for 4 periods. Advisory; the read size is what matters.
        let period_bytes = globals.period * frame_bytes;
        let shift = (usize::BITS - 1 - period_bytes.leading_zeros()).clamp(4, 16) as libc::c_int;
        let count = ((4 * period_bytes) >> shift).clamp(2, 0x7fff) as libc::c_int;
        let mut frag: libc::c_int = (count << 16) | shift;
        if unsafe { libc::ioctl(fd, SNDCTL_DSP_SETFRAGMENT, &mut frag) } != 0 {
            warn!("{}: SNDCTL_DSP_SETFRAGMENT: {}", path, io::Error::last_os_error());
        }

        let fmt = ioctl(SNDCTL_DSP_SETFMT, AFMT_S16_LE, "SNDCTL_DSP_SETFMT");
        assert_eq!(fmt, AFMT_S16_LE, "{}: format is {:#x}, not S16_LE", path, fmt);
        let ch = ioctl(
            SNDCTL_DSP_CHANNELS,
            globals.channels as libc::c_int,
            "SNDCTL_DSP_CHANNELS",
        );
        assert_eq!(ch as usize, globals.channels, "{}: {} channels", path, ch);
        let rate = ioctl(SNDCTL_DSP_SPEED, SENSE_RATE, "SNDCTL_DSP_SPEED");
        assert_eq!(rate, SENSE_RATE, "{}: rate {}", path, rate);
        println!("PCM rate: {}..{}", rate, rate);
        info!("{}: S16_LE, {} channels, {} Hz", path, ch, rate);

        SensePcm { dsp, frame_bytes }
    }

    /// Fill `buf` with whole frames. Short only when the driver times out
    /// part way (the trailing partial frame is dropped; the next data
    /// after a gap starts on a frame boundary, see realign in the spec).
    pub fn readi(&mut self, buf: &mut [i16]) -> Result<usize, ReadError> {
        let total = buf.len() * 2;
        let base = buf.as_mut_ptr() as *mut u8;
        let fd = self.dsp.as_raw_fd();
        let mut off = 0usize;
        while off < total {
            let n = unsafe {
                libc::read(
                    fd,
                    base.add(off) as *mut libc::c_void,
                    total - off,
                )
            };
            if n > 0 {
                off += n as usize;
                continue;
            }
            if n == 0 {
                panic!("Sense PCM: end of file");
            }
            let e = errno();
            if e == libc::EAGAIN {
                if off >= self.frame_bytes {
                    break;
                }
                return Err(ReadError::NoData);
            }
            return Err(ReadError::Other(e));
        }
        Ok(off / self.frame_bytes)
    }
}
