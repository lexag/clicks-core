use crate::audio;

use common::{
    command::{CommandError, ControlCommand},
    cue::{Beat, BeatEvent, Cue},
    status::{AudioSourceState, CombinedStatus},
    timecode::TimecodeInstant,
};

pub struct TimecodeSource {
    pub active: bool,
    pub frame_rate: usize,
    pub drop_frame: bool,
    pub color_framing: bool,
    pub external_clock: bool,
    volume: f32,
    frame_buffer: [f32; 8192],
    cue: Cue,
    current_time: TimecodeInstant,
    status: CombinedStatus,
}

impl Default for TimecodeSource {
    fn default() -> Self {
        Self {
            active: false,
            frame_rate: 25,
            drop_frame: false,
            color_framing: false,
            external_clock: false,
            volume: 1.0,
            frame_buffer: [0.0f32; 8192],
            cue: Cue::empty(),
            current_time: TimecodeInstant::new(25),
            status: CombinedStatus::default(),
        }
    }
}

impl TimecodeSource {
    pub fn new(frame_rate: usize) -> TimecodeSource {
        TimecodeSource {
            frame_rate,
            current_time: TimecodeInstant {
                frame_rate: frame_rate,
                h: 0,
                m: 0,
                s: 0,
                f: 0,
                frame_progress: 0,
            },
            ..Default::default()
        }
    }

    fn even_parity_bit(&self, mut data: u128) -> u128 {
        let mut parity = 0;

        while data != 0 {
            parity ^= data & 1;
            data >>= 1;
        }
        return parity;
    }

    fn generate_smpte_frame_bits(&self, user_bits: u32) -> u128 {
        let h0: u128 = (self.current_time.h.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let h1: u128 = (self.current_time.h.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let m0: u128 = (self.current_time.m.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let m1: u128 = (self.current_time.m.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let s0: u128 = (self.current_time.s.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let s1: u128 = (self.current_time.s.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let f0: u128 = (self.current_time.f.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let f1: u128 = (self.current_time.f.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let user_bits: u32 = 0;

        let mut t_enc: u128 = 0;

        // time values
        t_enc |= (f0 + 1) % self.frame_rate as u128;
        t_enc |= f1 << 8;
        t_enc |= s0 << 16;
        t_enc |= s1 << 24;
        t_enc |= m0 << 32;
        t_enc |= m1 << 40;
        t_enc |= h0 << 48;
        t_enc |= h1 << 56;

        // flags
        t_enc |= (self.drop_frame as u128) << 10;
        t_enc |= (self.color_framing as u128) << 11;
        t_enc |= (self.external_clock as u128) << 58;

        // user bits
        for i in 0..8 {
            t_enc |= ((user_bits & (0b1111 << i)) as u128) << 4 * i + 4;
        }

        // sync word
        t_enc |= 0b1011111111111100 << 64;

        let polarity_correction_bit: u128 = self.even_parity_bit(t_enc);
        if self.frame_rate == 25 {
            t_enc |= polarity_correction_bit << 59;
        } else {
            t_enc |= polarity_correction_bit << 27;
        }
        return t_enc;
    }

    fn generate_smpte_frame_buffer(&self, bits: u128, samples_per_bit: usize) -> [f32; 2048] {
        let mut buf = [0f32; 2048];
        let mut current_parity = 1;
        for bit_idx in 0..80 {
            let frame_bit = (0x1 << bit_idx) & bits;
            for sample_idx in 0..samples_per_bit as usize {
                if sample_idx == 0 {
                    current_parity *= -1;
                } else if sample_idx == samples_per_bit as usize / 2 && frame_bit != 0 {
                    current_parity *= -1;
                }
                buf[sample_idx + bit_idx as usize * samples_per_bit as usize] =
                    (current_parity as f32) * self.volume;
            }
        }
        return buf;
    }

    fn calculate_time_at_beat(&self, beat_idx: usize) -> TimecodeInstant {
        let mut time = TimecodeInstant {
            h: 0,
            m: 0,
            s: 0,
            f: 0,
            frame_progress: 0,
            frame_rate: self.frame_rate,
        };
        let mut time_off_us = 0_u64;
        for i in 0..beat_idx {
            for event in self.cue.get_beat(i).unwrap_or_default().events {
                match event {
                    BeatEvent::TimecodeEvent { h, m, s, f } => {
                        time.set_time(h, m, s, f);
                        time_off_us = 0;
                    }
                    _ => {}
                }
            }
            time_off_us += self.cue.get_beat(i).unwrap_or_default().length as u64;
        }
        time.add_us(time_off_us);
        return time;
    }
}

impl audio::source::AudioSource for TimecodeSource {
    fn get_status(&mut self, _c: &jack::Client, _ps: &jack::ProcessScope) -> AudioSourceState {
        return AudioSourceState::TimeStatus(self.current_time.clone());
    }
    fn command(&mut self, command: ControlCommand) -> Result<(), CommandError> {
        match command {
            ControlCommand::TransportZero => {
                self.current_time.set_time(0, 0, 0, 0);
                self.current_time.frame_progress = 0;

                for event in self.cue.get_beat(0).unwrap_or_default().events {
                    match event {
                        BeatEvent::TimecodeEvent { h, m, s, f } => {
                            self.current_time.set_time(h, m, s, f);
                            self.active = true;
                        }
                        _ => {}
                    }
                }
            }
            ControlCommand::TransportStop => {
                self.active = false;
            }
            ControlCommand::TransportStart => {
                self.active = true;
            }
            ControlCommand::TransportJumpBeat(beat_idx) => {
                self.current_time = self.calculate_time_at_beat(beat_idx);
            }
            ControlCommand::TransportSeekBeat(beat_idx) => {
                self.current_time = self.calculate_time_at_beat(beat_idx);
                self.current_time
                    .sub_us(self.status.transport.us_to_next_beat as u64)
            }
            ControlCommand::LoadCue(cue) => self.cue = cue.clone(),
            _ => {}
        }
        return Ok(());
    }

    fn send_buffer(
        &mut self,
        _c: &jack::Client,
        _ps: &jack::ProcessScope,
        status: CombinedStatus,
    ) -> Result<&[f32], jack::Error> {
        let sample_rate = _c.sample_rate() as u32;
        let last_cycle_frame = self.current_time.clone();
        self.status = status.clone();

        if self.active {
            self.current_time.add_progress(
                (_ps.n_frames() * self.frame_rate as u32 * 65536 / sample_rate) as u16,
            );
        }
        for event in self
            .cue
            .get_beat(status.beat_state().next_beat_idx)
            .unwrap_or(Beat::empty())
            .events
        {
            match event {
                BeatEvent::TimecodeEvent { h, m, s, f } => {
                    // if this cycle will run over the edge into next beat, we set the new timecode
                    // immediately AND restart the frame progress from 0. This is important, as
                    // otherwise, the frame time would change mid-frame, and confusion follows.
                    // Technically, this causes up to fps/48000 (<630us) seconds of inaccuracy, as the
                    // frame starts up to 1 whole cycle too early, but it is negligible, as the
                    // normal accuracy is only 1/fps (>33ms)
                    if (status.transport.us_to_next_beat as u32)
                        < (_ps.n_frames() as u32 * 1000000) / sample_rate
                    {
                        self.active = true;
                        self.current_time = TimecodeInstant {
                            frame_rate: self.frame_rate,
                            h: h as i16,
                            m: m as i16,
                            s: s as i16,
                            f: f as i16,
                            frame_progress: 0,
                        };
                    }
                }
                _ => {}
            }
        }

        if status.transport.running && self.active {
            // FIXME: will run slow(?) on some framerates where samples_per_bit gets truncated
            let samples_per_frame: usize = sample_rate as usize / self.frame_rate as usize;
            let samples_per_bit: usize = samples_per_frame / 80;

            let subframe_sample =
                self.current_time.frame_progress as u64 * samples_per_frame as u64 / 65536;

            if last_cycle_frame != self.current_time {
                self.frame_buffer.copy_within(
                    samples_per_frame as usize..2 * samples_per_frame as usize,
                    0,
                );

                // write next frame into next frame buffer
                let next_frame_bits = self.generate_smpte_frame_bits(0x0);
                let next_frame_buf = &self
                    .generate_smpte_frame_buffer(next_frame_bits, samples_per_bit)
                    [0..samples_per_frame];
                self.frame_buffer[samples_per_frame..2 * samples_per_frame]
                    .copy_from_slice(&next_frame_buf);
            }

            return Ok(&self.frame_buffer
                [subframe_sample as usize..subframe_sample as usize + _ps.n_frames() as usize]);
        }
        return Ok(&[0f32; 2048][0.._ps.n_frames() as usize]);
    }
}
