use crate::audio::{self, source::AudioSourceContext};

use common::{
    event::{EventCursor, EventDescription},
    local::status::{AudioSourceState, TimecodeState},
    mem::smpte::TimecodeInstant,
    protocol::request::ControlAction,
};

pub struct TimecodeSource {
    pub active: bool,
    pub frame_rate: u8,
    pub drop_frame: bool,
    pub color_framing: bool,
    pub external_clock: bool,
    volume: f32,
    frame_buffer: [f32; 8192],
    state: TimecodeState,
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
            state: TimecodeState { running: false, ltc: TimecodeInstant::new(25) }
        }
    }
}

impl TimecodeSource {
    pub fn new(frame_rate: u8) -> TimecodeSource {
        TimecodeSource {
            frame_rate,
            state: TimecodeState { running: false, ltc: TimecodeInstant::new(frame_rate) },
            ..Default::default()
        }
    }

    fn even_parity_bit(&self, mut data: u128) -> u128 {
        let mut parity = 0;

        while data != 0 {
            parity ^= data & 1;
            data >>= 1;
        }
        parity
    }

    fn generate_smpte_frame_bits(&self, user_bits: u32) -> u128 {
        let h0: u128 = (self.state.ltc.h.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let h1: u128 = (self.state.ltc.h.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let m0: u128 = (self.state.ltc.m.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let m1: u128 = (self.state.ltc.m.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let s0: u128 = (self.state.ltc.s.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let s1: u128 = (self.state.ltc.s.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let f0: u128 = (self.state.ltc.f.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let f1: u128 = (self.state.ltc.f.abs() / 10)
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
            t_enc |= ((user_bits & (0b1111 << i)) as u128) << (4 * i + 4);
        }

        // sync word
        t_enc |= 0b1011111111111100 << 64;

        let polarity_correction_bit: u128 = self.even_parity_bit(t_enc);
        if self.frame_rate == 25 {
            t_enc |= polarity_correction_bit << 59;
        } else {
            t_enc |= polarity_correction_bit << 27;
        }
        t_enc
    }

    fn generate_smpte_frame_buffer(&self, bits: u128, samples_per_bit: usize) -> [f32; 2048] {
        let mut buf = [0f32; 2048];
        let mut current_parity = 1;
        for bit_idx in 0..80 {
            let frame_bit = (0x1 << bit_idx) & bits;
            for sample_idx in 0..samples_per_bit {
                if sample_idx == 0 || (sample_idx == samples_per_bit / 2 && frame_bit != 0) {
                    current_parity *= -1;
                }
                buf[sample_idx + bit_idx as usize * samples_per_bit] =
                    (current_parity as f32) * self.volume;
            }
        }
        buf
    }

    fn calculate_time_at_beat(&self, ctx: &AudioSourceContext, beat_idx: u16) -> TimecodeInstant {
        let mut time = TimecodeInstant {
            h: 0,
            m: 0,
            s: 0,
            f: 0,
            frame_progress: 0,
            frame_rate: self.frame_rate,
        };
        let mut time_off_us = 0_u64;
        let mut cursor = EventCursor::new(&ctx.cue.events);
        for i in 0..beat_idx {
            while cursor.at_or_before(beat_idx)
                && let Some(event) = cursor.get_next()
            {
                if let Some(EventDescription::TimecodeEvent { time: new_time }) = event.event {
                    time = new_time;
                    time_off_us = 0;
                }
            }
            time_off_us += ctx.cue.get_beat(i).unwrap_or_default().length as u64;
        }
        time.add_us(time_off_us);
        time
    }
}

impl audio::source::AudioSource for TimecodeSource {
    fn get_status(&mut self, _ctx: &AudioSourceContext) -> AudioSourceState {
        AudioSourceState::TimeStatus(self.state)
    }

    fn command(&mut self, ctx: &AudioSourceContext, command: ControlAction) {
        match command {
            ControlAction::TransportZero => {
                self.state.ltc.set_time(0, 0, 0, 0);
                self.state.ltc.frame_progress = 0;
            }
            ControlAction::TransportStop => {
                self.active = false;
            }
            ControlAction::TransportStart => {
                self.active = true;
            }
            ControlAction::TransportJumpBeat(beat_idx) => {
                self.state.ltc = self.calculate_time_at_beat(ctx, beat_idx);
            }
            ControlAction::TransportSeekBeat(beat_idx) => {
                self.state.ltc = self.calculate_time_at_beat(ctx, beat_idx);
                self.state.ltc
                    .sub_us(ctx.beat.us_to_next_beat as u64)
            }
            _ => {}
        }
    }

    fn send_buffer(&mut self, ctx: &AudioSourceContext) -> Result<&[f32], jack::Error> {
        let last_cycle_frame = self.state.ltc;

        if self.active {
            self.state.ltc
                .add_progress((ctx.frame_size * self.frame_rate as usize * 65536 / ctx.sample_rate) as u16);
        }

        if !ctx.transport.running || !self.active {
            return Ok(self.silence(ctx.frame_size));
        }

        // FIXME: will run slow(?) on some framerates where samples_per_bit gets truncated
        let samples_per_frame: usize = ctx.sample_rate / self.frame_rate as usize;
        let samples_per_bit: usize = samples_per_frame / 80;

        let subframe_sample =
            self.state.ltc.frame_progress as u64 * samples_per_frame as u64 / 65536;

        if last_cycle_frame != self.state.ltc {
            self.frame_buffer.copy_within(
                samples_per_frame..2 * samples_per_frame,
                0,
            );

            // write next frame into next frame buffer
            let next_frame_bits = self.generate_smpte_frame_bits(0x0);
            let next_frame_buf = &self
                .generate_smpte_frame_buffer(next_frame_bits, samples_per_bit)
                [0..samples_per_frame];
            self.frame_buffer[samples_per_frame..2 * samples_per_frame]
                .copy_from_slice(next_frame_buf);
        }

        Ok(&self.frame_buffer
            [subframe_sample as usize..subframe_sample as usize + ctx.frame_size])
    }

    fn event_occured(&mut self, _ctx: &AudioSourceContext, _event: common::event::Event) {}

    fn event_will_occur(&mut self, _ctx: &AudioSourceContext, event: common::event::Event) {
        if let Some(EventDescription::TimecodeEvent { time }) = event.event {
            // if this cycle will run over the edge into next beat, we set the new timecode
            // immediately AND restart the frame progress from 0. This is important, as
            // otherwise, the frame time would change mid-frame, and confusion follows.
            // Technically, this causes up to fps/48000 (<630us) seconds of inaccuracy, as the
            // frame starts up to 1 whole cycle too early, but it is negligible, as the
            // normal accuracy is only 1/fps (>33ms)
            self.active = true;
            self.state.ltc = time;
        }
    }
}
