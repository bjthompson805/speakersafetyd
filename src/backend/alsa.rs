// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) The Asahi Linux Contributors
//! ALSA backend (Linux): the upstream code, moved behind the backend API.

use std::ffi::CString;

use alsa::mixer::MilliBel;
use log::info;

use super::{CtlId, CtlKind, ReadError};
use crate::types::Globals;

pub struct Card {
    ctl: alsa::ctl::Ctl,
    device: String,
}

impl Card {
    pub fn open(maker: &str, model: &str) -> Card {
        let maker_titlecase = maker[0..1].to_ascii_uppercase() + &maker[1..];
        let device = format!("hw:{}{}", maker_titlecase, model.to_ascii_uppercase());
        info!("Device: {}", device);
        let ctl = match alsa::ctl::Ctl::new(&device, false) {
            Ok(ctl) => ctl,
            Err(e) => panic!("{}: Could not open sound card! Error: {}", device, e),
        };
        Card { ctl, device }
    }
}

/**
    Struct with fields necessary for manipulating an ALSA elem.
*/
pub struct Elem {
    elem_name: String,
    id: alsa::ctl::ElemId,
    val: alsa::ctl::ElemValue,
}

impl Elem {
    pub fn new(cid: CtlId, card: &Card) -> Elem {
        let t = match cid.kind {
            CtlKind::VSense | CtlKind::ISense => alsa::ctl::ElemType::Boolean,
            _ => alsa::ctl::ElemType::Integer,
        };
        let val = alsa::ctl::ElemValue::new(t)
            .unwrap_or_else(|_| panic!("Could not open a handle to an element!"));
        let mut e = Elem {
            elem_name: cid.alsa_name,
            id: alsa::ctl::ElemId::new(alsa::ctl::ElemIface::Mixer),
            val,
        };
        let cname = CString::new(e.elem_name.clone()).unwrap();
        e.id.set_name(cname.as_c_str());
        e.val.set_id(&e.id);
        if let Err(err) = card.ctl.elem_lock(&e.id) {
            panic!(
                "Could not lock elem {}. alsa-lib error: {:?}",
                e.elem_name, err
            );
        }
        e.read(card);
        e
    }

    pub fn name(&self) -> &str {
        &self.elem_name
    }

    fn read(&mut self, card: &Card) {
        if let Err(e) = card.ctl.elem_read(&mut self.val) {
            panic!(
                "Could not read elem value {}. alsa-lib error: {:?}",
                self.elem_name, e
            );
        }
    }

    fn write(&self, card: &Card) {
        if let Err(e) = card.ctl.elem_write(&self.val) {
            panic!(
                "Could not write elem value {}. alsa-lib error: {:?}",
                self.elem_name, e
            );
        }
    }

    pub fn read_int(&mut self, card: &Card) -> i32 {
        self.read(card);
        self.val
            .get_integer(0)
            .unwrap_or_else(|| panic!("Could not read {}", self.elem_name))
    }

    pub fn write_int(&mut self, card: &Card, value: i32) {
        self.val
            .set_integer(0, value)
            .unwrap_or_else(|| panic!("Could not set {}", self.elem_name));
        self.write(card);
    }

    pub fn read_bool(&mut self, card: &Card) -> bool {
        self.read(card);
        self.val
            .get_boolean(0)
            .unwrap_or_else(|| panic!("Could not read {}", self.elem_name))
    }

    pub fn write_bool(&mut self, card: &Card, value: bool) {
        self.val
            .set_boolean(0, value)
            .unwrap_or_else(|| panic!("Could not set {}", self.elem_name));
        self.write(card);
    }

    /// (min, max) in mB.
    pub fn db_range(&mut self, card: &Card) -> (i64, i64) {
        match card.ctl.get_db_range(&self.id) {
            Ok((a, b)) => (a.0, b.0),
            Err(e) => panic!(
                "Could not get elem db range {}. alsa-lib error: {:?}",
                self.elem_name, e
            ),
        }
    }

    /// mB to raw value, rounding down.
    pub fn from_db(&mut self, card: &Card, mb: i64) -> i32 {
        match card
            .ctl
            .convert_from_db(&self.id, MilliBel(mb), alsa::Round::Floor)
        {
            Ok(v) => v as i32,
            Err(e) => panic!(
                "Could not convert MilliBel {} to int! alsa-lib error: {:?}",
                mb, e
            ),
        }
    }

    /// Raw value to mB.
    pub fn to_db(&self, card: &Card, val: i32) -> i64 {
        match card.ctl.convert_to_db(&self.id, val.into()) {
            Ok(mb) => mb.0,
            Err(e) => panic!(
                "Could not convert val {} to dB! alsa-lib error: {:?}",
                val, e
            ),
        }
    }
}

pub struct SensePcm {
    pcm: alsa::pcm::PCM,
}

impl SensePcm {
    pub fn open(card: &Card, globals: &Globals) -> SensePcm {
        let dev = format!("{},{}", card.device, globals.visense_pcm);
        let chans: u32 = globals.channels.try_into().unwrap();
        let pcm = alsa::pcm::PCM::new(&dev, alsa::Direction::Capture, false).unwrap();
        {
            let params = alsa::pcm::HwParams::any(&pcm).unwrap();
            let rate_max = params.get_rate_max().unwrap();
            let rate_min = params.get_rate_min().unwrap();
            println!("PCM rate: {}..{}", rate_min, rate_max);
            params.set_channels(chans).unwrap();
            params.set_rate(rate_min, alsa::ValueOr::Nearest).unwrap();
            params.set_format(alsa::pcm::Format::s16()).unwrap();
            params.set_access(alsa::pcm::Access::RWInterleaved).unwrap();
            pcm.hw_params(&params).unwrap();
        }
        SensePcm { pcm }
    }

    /// Blocking read of interleaved frames; returns frames read.
    pub fn readi(&mut self, buf: &mut [i16]) -> Result<usize, ReadError> {
        let io = self.pcm.io_i16().unwrap();
        io.readi(buf).map_err(|e| {
            if e.errno() == libc::ESTRPIPE {
                ReadError::Suspended
            } else {
                ReadError::Other(e.errno())
            }
        })
    }
}
