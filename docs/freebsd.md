# speakersafetyd on FreeBSD (M5 MacBook Air 15", J815)

The daemon's algorithm is unchanged: the thermal model, the per-speaker
ceilings from `conf/apple/j815.conf`, group linking, the unlock heartbeat
once per sense period, cold/warm boot via the flag file, and panic on any
error so the kernel relocks the speakers. Only the I/O layer differs. The
kernel contract is m1n1-env `docs/speaker-interface.md` (apple_macaudio
`apple_macaudio_spk.c`, `tas2764.c`, freebsd-src-m5 branch `m5-speakers`).

## Layout

| File | Role |
|---|---|
| `src/backend/mod.rs` | the backend API (`Card`, `Elem`, `SensePcm`, `CtlId`/`CtlKind`, `ReadError`) |
| `src/backend/alsa.rs` | Linux: the upstream alsa-rs code, moved, `cfg(target_os = "linux")` |
| `src/backend/freebsd.rs` | FreeBSD: sysctl(3), `/dev/spkctlN`, OSS `/dev/dspN` |
| `src/types.rs`, `src/main.rs` | shared; call the backend instead of alsa-rs |
| `rc.d/speakersafetyd` | rc(8) script, daemon(8) with restart |

`alsa` is a Linux-only dependency; nothing ALSA is built on FreeBSD.
`uclamp` (Linux `sched_setattr` utilization clamp) is a logged no-op on
FreeBSD.

## How each operation maps

| Upstream (ALSA) | FreeBSD backend |
|---|---|
| machine from `/proc/device-tree/compatible` | first word of `hw.fdt.compatible` (`apple,j815`), trailing letters trimmed as upstream; selects `<conf>/apple/j815.conf` |
| `Ctl::new("hw:AppleJ815")` | discovery: the speaker pcm is the unit with `dev.pcm.N.speaker.amps`; the sense pcm is the unit whose `dev.pcm.N.%desc` is `Apple M5 audio (speaker sense)`. Then `open("/dev/spkctl<spk>", O_RDONLY)`, held until exit |
| `elem_lock` on each control | implied by holding `/dev/spkctlN`; checked at startup: `owner_pid == getpid()` |
| `<Prefix> VSENSE/ISENSE Switch` | `dev.pcm.N.speaker.<amp>.vsense` / `.isense`, written 1 and read back |
| `<Prefix> Amp Gain Volume`, dB range, `convert_from_db(max, Floor)` | `.amp_gain`; range from `.amp_gain_min/_max`, mB = `amp_gain_db_min + v * amp_gain_db_step` |
| `<Prefix> Speaker Volume`, `convert_from_db(lvl, Floor)`, write, read back | `.volume`; `v = floor((mB - volume_db_min) / volume_db_step)` clamped to `[volume_min, volume_max]`, `volume_max` re-read on every conversion (it is 1 while locked, 161 unlocked) |
| `Speaker Sample Rate` | `dev.pcm.N.speaker.sample_rate` |
| `Speaker Volume Unlock` = `UNLOCK_MAGIC` | `sysctlbyname("dev.pcm.N.speaker.unlock", 0xdec1be15)`; any errno (EPERM, EBUSY, ETIMEDOUT) panics, as any alsa write error does upstream |
| `PCM::new("hw:..,2", Capture)`, S16, 12 ch | `open("/dev/dsp<sense>", O_RDONLY)`, `SNDCTL_DSP_SETFRAGMENT` (advisory), `SETFMT AFMT_S16_LE`, `CHANNELS 12`, `SPEED 48000`; each returned value must equal the request (bit-perfect channel) |
| `readi(buf)` | `read()` looped until `period * 24` bytes |
| ESTRPIPE -> reopen PCM | not produced on FreeBSD (no suspend yet); the code path is kept |

`amp` is the conf section name (`Speaker/Left Woofer 1`) lower-cased with
non-alphanumerics as `_` (`left_woofer_1`); the backend checks that the
node's `.name` equals the conf name.

## Where behaviour differs from Linux, and why

1. **Idle sense reads.** Linux sets the sense PCM's wait time to infinity,
   so `readi` blocks until the speakers play. FreeBSD's `read()` returns
   EAGAIN after `hw.snd.timeout` (5 s) without data. The daemon then
   resets sense history (a gap), writes the unlock magic and reads again.
   The ping is harmless while stopped (the lease counts down only while
   `playing`), and it means a lost lock still turns into a panic within
   5 s even without sense data. `last_update` is not advanced, so the
   first period after the gap runs the model's cool-down catch-up exactly
   as after a long blocking read on Linux.
2. **Short reads.** If the timeout hits part way through a period, the
   whole frames read so far are returned (a partial trailing frame is
   dropped; the next data after a gap starts on a frame boundary, the
   kernel's realign). Upstream already handles `read != period` (warns).
3. **Startup checks** that ALSA has no equivalent for, each a panic:
   `owner_pid` is not us; `unlock_magic` differs from the daemon's
   constant; `failed != 0` (the kernel refuses speakers until reload
   anyway); `interface != 0` (the lease interface would EBUSY every ping).
   Opening `/dev/spkctlN` while another process holds it panics with the
   holder's pid (EBUSY), where ALSA's elem_lock would fail similarly.
4. **`visense_pcm`** in the conf is the ALSA device number; on FreeBSD the
   sense unit is discovered and the conf value is only logged.
5. **Sample rate** is requested as 48000 (the only link rate, `MAS_RATE`)
   instead of the PCM's `rate_min`; on this hardware they are the same.
6. **Paths.** Flag file `/var/run/speakersafetyd/speakersafetyd.flag`
   (cleared at boot like `/run`); default config base
   `/usr/local/share/speakersafetyd/conf`.
7. **uclamp** (j815.conf `uclamp_max = 64`) is not applied: FreeBSD has no
   utilization clamp. Logged at startup.
8. **Runs as root** (the owner device is root 0600 and sysctl writes need
   root); upstream's systemd unit uses a DynamicUser in group `audio`.
9. **`--muted`** (new, both OSes): every Speaker Volume write is 0 (mute)
   instead of the model's level; everything else (lock, pings, sense,
   model, read-back) runs unchanged. For bring-up only.

## Build and install

    cargo build --release
    make install-freebsd        # /usr/local/sbin, /usr/local/share/speakersafetyd/conf/apple, /usr/local/etc/rc.d
    sysrc speakersafetyd_enable=YES
    service speakersafetyd start

rc.conf knobs: `speakersafetyd_flags` (default `-m 7`, passed to the
daemon, not to daemon(8)), `speakersafetyd_config`. daemon(8) restarts
it 1 s after any exit (`-r -R 1`) and sends its output to syslog
(`-S -T speakersafetyd`, so `/var/log/messages`). Stop: `service
speakersafetyd stop` (SIGTERM; the owner device closes and the kernel
relocks).

## First hardware test (speakers inaudible until step 6)

Prerequisites: kernel with `apple_macaudio` + `tas2764` from `m5-speakers`,
daemon installed but not enabled, nothing else playing on the speaker pcm.
Unit numbers below assume speaker pcm1 / sense pcm2; the daemon logs the
real ones. Each step says what to check; stop at the first mismatch.

1. **Kernel state, no daemon.**
   `sysctl dev.pcm.1.speaker` -> `unlock: 0`, `interface: 0`,
   `owner_pid: 0`, `failed: 0`, `amps: 6`, every `<amp>.volume_max: 1`.
   `sysctl hw.fdt.compatible` starts with `apple,j815`.
2. **Muted daemon, idle.** As root, in the foreground:
   `speakersafetyd -c /usr/local/share/speakersafetyd/conf --muted -vv -m 7`.
   Expect: `Machine: apple,j815`, `Speaker pcm: pcm1, sense pcm: pcm2`,
   `/dev/dsp2: S16_LE, 12 channels, 48000 Hz`, six speaker blocks with
   `Amp gain: 11 dBV`. In another shell: `owner_pid` = daemon pid, `unlock: 1`,
   every `<amp>.vsense/isense: 1`, `amp_gain: 0`, `volume: 0`,
   `volume_max: 161`. `Sense PCM idle` every ~5 s (debug), no panic.
   As a non-owner root: `sysctl dev.pcm.1.speaker.left_woofer_1.volume=10`
   must fail with EPERM.
3. **Muted daemon, playing silence.** Keep the daemon running. Play
   digital silence to the speakers with a player that sets the format
   (`pkg install sox`): `sox -n -b 16 -r 48000 -c 2 -t oss /dev/dsp1 trim 0 10`.
   Do not `dd /dev/zero` into the device: its default format is U8, where
   0 is full-scale negative DC. During it: `playing: 1`,
   `sample_rate: 48000`, `unlock: 1` the whole time (the pings hold it),
   `amps_on: 1`, volumes still 0. The daemon logs `Sample rate: 48000`
   and per-period `SENSE ... power=` lines near 0. Restart with
   `--sense-capture /tmp/sense.raw` to keep 20 s of raw 12 x S16 frames
   for offline checks (alignment, zeros vs noise).
4. **Fail-safe: kill.** While step 3's playback runs, `kill -9` the
   daemon. Immediately: `owner_pid: 0`, `unlock: 0`, `volume_max: 1`,
   `amps_on: 0`. Playback continues (silence), no error to `dd`.
5. **Fail-safe: missed heartbeat.** Start the muted daemon and a playback
   again; `kill -STOP` the daemon for 1 s, then `kill -CONT`. The kernel
   must have relocked (`unlock: 0` while stopped) and the daemon must
   panic on its next ping (`ETIMEDOUT`). Under rc.d, daemon(8) restarts
   it (warm boot: flag file present, gains start at the worst case).
6. **Audible, low.** Only after 1-5 pass: run without `--muted`, set the
   pcm1 mixer low (`mixer -d pcm1 vol=0.1`), play a quiet test tone.
   Volumes read back at the ceilings (woofers 153 = -24 dB, tweeters
   137 = -32 dB on cold boot). Watch the THERMAL lines. Then enable the
   rc.d service.
