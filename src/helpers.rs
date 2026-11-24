// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) The Asahi Linux Contributors

use configparser::ini::Ini;

/**
    Wrapper around configparser::ini::Ini.getint()
    to safely unwrap the Result<Option<i64>, E> returned by
    it.
*/
pub fn parse_int<T: TryFrom<i64>>(config: &Ini, section: &str, key: &str) -> T
where
    <T as TryFrom<i64>>::Error: std::fmt::Debug,
{
    config
        .getint(section, key)
        .unwrap_or_else(|_| panic!("{}/{}: Invalid value", section, key))
        .unwrap_or_else(|| panic!("{}/{}: Missing key", section, key))
        .try_into()
        .expect("{}/{}: Out of bounds")
}

pub fn parse_opt_int<T: TryFrom<i64>>(config: &Ini, section: &str, key: &str) -> Option<T>
where
    <T as TryFrom<i64>>::Error: std::fmt::Debug,
{
    config
        .getint(section, key)
        .unwrap_or_else(|_| panic!("{}/{}: Invalid value", section, key))
        .map(|a| a.try_into().expect("{}/{}: Out of bounds"))
}

/**
    Wrapper around configparser::ini::Ini.getfloat()
    to safely unwrap the Result<Option<f64>, E> returned by
    it.
*/
pub fn parse_float(config: &Ini, section: &str, key: &str) -> f32 {
    let val = config
        .getfloat(section, key)
        .unwrap_or_else(|_| panic!("{}/{}: Invalid value", section, key))
        .unwrap_or_else(|| panic!("{}/{}: Missing key", section, key)) as f32;

    assert!(val.is_finite());
    val
}

/**
    Wrapper around configparser::ini::Ini.getfloat()
    to safely unwrap the Result<Option<f64>, E> returned by
    it.
*/
pub fn parse_string(config: &Ini, section: &str, key: &str) -> String {
    config
        .get(section, key)
        .unwrap_or_else(|| panic!("{}/{}: Missing key", section, key))
}
