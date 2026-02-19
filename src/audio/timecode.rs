use crate::audio::{self, source::AudioSourceContext};

use common::{
    event::{EventCursor, EventDescription},
    local::status::{AudioSourceState, TimecodeState},
    mem::smpte::TimecodeInstant,
    protocol::request::ControlAction,
};

pub struct TimecodeSource {
    pub frame_rate: u8,
    pub drop_frame: bool,
    pub color_framing: bool,
    pub external_clock: bool,
    volume: f32,
    frame_buffer: [f32; 8192],
    state: TimecodeState,
    last_cycle_frame: TimecodeInstant,
}

impl Default for TimecodeSource {
    fn default() -> Self {
        Self {
            frame_rate: 25,
            drop_frame: false,
            color_framing: false,
            external_clock: false,
            volume: 0.5,
            frame_buffer: [0.0f32; 8192],
            state: TimecodeState {
                running: false,
                ltc: TimecodeInstant::new(25),
            },
            last_cycle_frame: TimecodeInstant::new(25),
        }
    }
}

impl TimecodeSource {
    pub fn new(frame_rate: u8) -> TimecodeSource {
        TimecodeSource {
            frame_rate,
            state: TimecodeState {
                running: false,
                ltc: TimecodeInstant::new(frame_rate),
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

                let idx = sample_idx + bit_idx as usize * samples_per_bit;
                buf[idx] = (current_parity as f32) * self.volume;
            }
        }

        let mut lp_buffer = [0_f32; 2048];
        self.low_pass(&buf, &mut lp_buffer);

        lp_buffer
    }

    fn low_pass(&self, buf: &[f32], out: &mut [f32]) {
        const LP_WIDTH: usize = 2;
        for idx in LP_WIDTH..buf.len() - LP_WIDTH {
            let mut cumsum = 0.0;
            for offs_idx in idx - LP_WIDTH..idx + LP_WIDTH {
                cumsum += buf[offs_idx] * (LP_WIDTH - (idx.abs_diff(offs_idx))) as f32;
            }
            cumsum /= LP_WIDTH as f32 * (LP_WIDTH as f32 + 1.0) / 2.0;
            out[idx] = cumsum;
        }
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
        let mut cursor = EventCursor::new(&ctx.cue.events);
        for i in 0..beat_idx {
            while cursor.at_or_before(beat_idx)
                && let Some(event) = cursor.get_next()
            {
                if let Some(EventDescription::TimecodeEvent { time: new_time }) = event.event
                    && event.location == i
                {
                    time = new_time;
                }
            }
            time.add_us(ctx.cue.get_beat(i).unwrap_or_default().length as u64);
        }
        time
    }

    pub fn advance_by_samples(&mut self, samples: usize, sample_rate: usize) {
        self.state
            .ltc
            .add_progress((samples * self.frame_rate as usize * 65536 / sample_rate) as u16);
    }

    fn calculate_frame_overlap(&mut self, sample_rate: usize) -> u64 {
        // FIXME: will run slow(?) on some framerates where samples_per_bit gets truncated
        let samples_per_frame: usize = sample_rate / self.frame_rate as usize;
        let samples_per_bit: usize = samples_per_frame / 80;

        let subframe_sample =
            self.state.ltc.frame_progress as u64 * samples_per_frame as u64 / 65536;

        if self.last_cycle_frame != self.state.ltc {
            self.frame_buffer
                .copy_within(samples_per_frame..2 * samples_per_frame, 0);

            // write next frame into next frame buffer
            let next_frame_bits = self.generate_smpte_frame_bits(0x0);
            let next_frame_buf = &self
                .generate_smpte_frame_buffer(next_frame_bits, samples_per_bit)
                [0..samples_per_frame];
            self.frame_buffer[samples_per_frame..2 * samples_per_frame]
                .copy_from_slice(next_frame_buf);

            //// interpolate 2 last samples at the change point from current frame buffer to next frame
            //// buffer
            //self.frame_buffer[samples_per_frame - 2] = (self.frame_buffer[samples_per_frame] * 2.0
            //    + self.frame_buffer[samples_per_frame - 3] * 3.0)
            //    / 5.0;
            //self.frame_buffer[samples_per_frame - 1] = (self.frame_buffer[samples_per_frame] * 3.0
            //    + self.frame_buffer[samples_per_frame - 2] * 2.0)
            //    / 5.0;
        }

        subframe_sample
    }

    fn frame(&mut self, frame_size: usize, sample_rate: usize) -> &[f32] {
        if self.state.running {
            self.advance_by_samples(frame_size, sample_rate);
        }

        let subframe_sample = self.calculate_frame_overlap(sample_rate);

        self.last_cycle_frame = self.state.ltc;
        &self.frame_buffer[subframe_sample as usize..subframe_sample as usize + frame_size]
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
                self.state.running = false;
            }
            ControlAction::TransportStart => {
                self.state.running = true;
            }
            ControlAction::TransportJumpBeat(beat_idx) => {
                if !ctx.transport.running {
                    self.state.ltc = self.calculate_time_at_beat(ctx, beat_idx);
                }
            }
            ControlAction::TransportSeekBeat(beat_idx) => {
                if !ctx.transport.running {
                    self.state.ltc = self.calculate_time_at_beat(ctx, beat_idx);
                    self.state.ltc.sub_us(ctx.beat.us_to_next_beat as u64)
                }
            }
            _ => {}
        }
    }

    fn send_buffer(&mut self, ctx: &AudioSourceContext) -> Result<&[f32], jack::Error> {
        if !ctx.transport.running || !self.state.running {
            return Ok(self.silence(ctx.frame_size));
        }

        Ok(self.frame(ctx.frame_size, ctx.sample_rate))
    }

    fn event_occured(&mut self, _ctx: &AudioSourceContext, _event: common::event::Event) {}

    fn event_will_occur(&mut self, ctx: &AudioSourceContext, event: common::event::Event) {
        if let Some(EventDescription::TimecodeEvent { time }) = event.event {
            if ctx.beat.next_beat_idx != event.location {
                return;
            }
            // if this cycle will run over the edge into next beat, we set the new timecode
            // immediately AND restart the frame progress from 0. This is important, as
            // otherwise, the frame time would change mid-frame, and confusion follows.
            // Technically, this causes up to fps/48000 (<630us) seconds of inaccuracy, as the
            // frame starts up to 1 whole cycle too early, but it is negligible, as the
            // normal accuracy is only 1/fps (>33ms)
            self.state.ltc = time;
            // FIXME: This is temporary solution for stopping timecode. Needs proper support from
            // clicks-common. When a timecode event is set above 24 hours, it will stop
            self.state.running = time.h <= 24;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::audio::timecode::TimecodeSource;
    use std::f32;

    fn rise_fall_time(buf: &[f32]) -> (f32, f32, f32) {
        let peak_amplitude = buf
            .iter()
            .max_by(|x, y| x.abs().partial_cmp(&y.abs()).unwrap())
            .unwrap();
        let max_fluctuate = peak_amplitude.abs() * 0.05;

        let mut min_time = 10000000.0;
        let mut max_time = 0.0;
        let mut avg_sum = 0.0;
        let mut avg_cnt = 0;
        // Find left side of crossing
        // i.e. left of point is flat (approx same),
        // and right of point is not flat (not approx same)
        //
        for left in 1..buf.len() - 1 {
            if f32::abs(buf[left] - buf[left - 1]) < max_fluctuate
                && f32::abs(buf[left] - buf[left + 1]) > max_fluctuate
            {
                // find right side of crossing
                // i.e. right of point is flat (approx same),
                // and left of point is not flat (not approx same)
                for right in left..left + 10 {
                    if f32::abs(buf[right] - buf[right + 1]) < max_fluctuate
                        && f32::abs(buf[right] - buf[right - 1]) > max_fluctuate
                    {
                        // FIXME: locked to 48kHz
                        let crossing_time = 1000.0 * (right - left) as f32 / 48.0;
                        min_time = f32::min(min_time, crossing_time);
                        max_time = f32::max(max_time, crossing_time);
                        avg_sum += crossing_time;
                        avg_cnt += 1;
                        break;
                    }
                }
            }
        }

        (min_time, avg_sum / avg_cnt as f32, max_time)
    }

    #[test]
    fn smpte_ltc_spec() {
        // EBU Time-And-Control Code FOR TELEVISION TAPE-RECORDINGS
        // https://tech.ebu.ch/docs/tech/tech3097.pdf

        use super::*;

        let mut tc = TimecodeSource::new(25);
        tc.state.running = true;

        const FRAME_SIZE: usize = 256;
        const NUM_FRAMES: usize = 1000;
        let mut frame = [0_f32; FRAME_SIZE * NUM_FRAMES];
        for i in 0..NUM_FRAMES {
            let _ = &frame[i * FRAME_SIZE..(i + 1) * FRAME_SIZE]
                .copy_from_slice(tc.frame(FRAME_SIZE, 48000));
        }

        // println!("{:?}", frame);

        // rise and fall time between 40 and 65 us
        let (min, avg, max) = rise_fall_time(&frame);

        println!("Rise/fall time report:");
        println!("min: {}, avg: {}, max: {}", min, avg, max);

        assert!(
            (40.0..65.0).contains(&avg),
            "Rise/fall time avg is {}, should be 40 -- 65",
            avg
        );
        assert!(
            (40.0..65.0).contains(&min),
            "Rise/fall time min is {}, should be >= 40",
            min
        );
        assert!(
            (40.0..65.0).contains(&max),
            "Rise/fall time max is {}, should be <= 65",
            max
        );
    }

    #[test]
    #[ignore = "this test produces a file output"]
    fn export_ltc_as_wav() {
        const NUM_SECS: usize = 100;
        const SAMPLE_RATE: usize = 48000;
        const FRAME_SIZE: usize = 256;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut all_zeroes = true;

        let mut writer = hound::WavWriter::create("target/debug/ltc.wav", spec).unwrap();
        let mut tc = TimecodeSource::new(25);
        tc.state.running = true;
        for _ in (0..SAMPLE_RATE * NUM_SECS).step_by(FRAME_SIZE) {
            for sample in tc.frame(FRAME_SIZE, SAMPLE_RATE) {
                if sample.abs() > 0.001 {
                    all_zeroes = false;
                }
                writer
                    .write_sample((sample * 65536.0 / 4.0) as i16)
                    .unwrap();
            }
        }
        writer.finalize().unwrap();

        assert!(!all_zeroes, "Export resulted in a silent file");
    }

    #[test]
    fn advance() {
        use super::*;

        let mut tc = TimecodeSource::new(25);
        assert_eq!(tc.state.ltc, TimecodeInstant::new(25));
        tc.advance_by_samples(48000 / 50, 48000);
        tc.advance_by_samples(48000 / 50, 48000);

        assert_eq!(tc.state.ltc.s, 0);
        assert_eq!(tc.state.ltc.f, 1);
    }
}
