use crate::audio::{self, source::AudioSourceContext};

use common::{
    event::{EventCursor, EventDescription},
    local::status::{AudioSourceState, TimecodeState},
    mem::smpte::{TimecodeInstant, TimecodeProperties, TimecodeUserBitFormat},
    protocol::{
        message::{Message, SmallMessage},
        request::ControlAction,
    },
};

pub struct TimecodeSource {
    pub properties: TimecodeProperties,
    volume: f32,
    frame_buffer: [f32; 8192],
    state: TimecodeState,
    last_cycle_frame: TimecodeInstant,
    sample_rate: usize,
    subframe_sample: u64,
}

impl Default for TimecodeSource {
    fn default() -> Self {
        Self {
            properties: TimecodeProperties::default(),
            volume: 0.5,
            frame_buffer: [0.0f32; 8192],
            state: TimecodeState {
                running: false,
                ltc: TimecodeInstant::new(25),
            },
            last_cycle_frame: TimecodeInstant::new(25),
            sample_rate: 48000,
            subframe_sample: 0,
        }
    }
}

impl TimecodeSource {
    pub fn new(sample_rate: usize) -> TimecodeSource {
        TimecodeSource {
            state: TimecodeState {
                running: false,
                ltc: TimecodeInstant::new(25),
            },
            sample_rate,
            ..Default::default()
        }
    }

    pub fn init(sample_rate: usize, properties: TimecodeProperties) -> TimecodeSource {
        let mut tc = TimecodeSource {
            state: TimecodeState {
                running: false,
                ltc: TimecodeInstant::new(25),
            },
            properties,
            sample_rate,
            ..Default::default()
        };
        tc.state.running = true;
        tc.preload_frame_buffer();
        tc
    }

    fn frame_rate(&self) -> u8 {
        self.state.ltc.frame_rate
    }

    fn even_parity_bit(&self, mut data: u128) -> u128 {
        let mut parity = 0;

        while data != 0 {
            parity ^= data & 1;
            data >>= 1;
        }
        parity
    }

    fn generate_smpte_frame_bits(&self, time: TimecodeInstant) -> u128 {
        let h0: u128 = (time.h.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let h1: u128 = (time.h.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let m0: u128 = (time.m.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let m1: u128 = (time.m.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let s0: u128 = (time.s.abs() % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let s1: u128 = (time.s.abs() / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let f0: u128 = ((time.f.abs() + self.properties.frame_offset as i8) % 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");
        let f1: u128 = ((time.f.abs() + self.properties.frame_offset as i8) / 10)
            .try_into()
            .expect("u16 -> u128 cannot fail.");

        let mut t_enc: u128 = 0;

        // time values
        t_enc |= f0;
        t_enc |= f1 << 8;
        t_enc |= s0 << 16;
        t_enc |= s1 << 24;
        t_enc |= m0 << 32;
        t_enc |= m1 << 40;
        t_enc |= h0 << 48;
        t_enc |= h1 << 56;

        // flags
        t_enc |= (self.properties.drop_frame as u128) << 10;
        t_enc |= (self.properties.color_framing as u128) << 11;
        t_enc |= (self.properties.use_wall_time as u128) << 58;

        let (polarity_idx, bin_group_0_idx, bin_group_2_idx) = if self.frame_rate() == 25 {
            (59, 27, 43)
        } else {
            (27, 43, 59)
        };

        // user bit format
        t_enc |= (if self.properties.user_bit_format == TimecodeUserBitFormat::Reserved11
            || self.properties.user_bit_format == TimecodeUserBitFormat::EightBitLittleEndian
        {
            1_u128
        } else {
            0_u128
        }) << bin_group_0_idx;

        t_enc |= (if self.properties.user_bit_format == TimecodeUserBitFormat::Reserved11
            || self.properties.user_bit_format == TimecodeUserBitFormat::DateTimezone
        {
            1_u128
        } else {
            0_u128
        }) << bin_group_2_idx;

        // user bits
        let user_bits = u32::from_ne_bytes(self.properties.user_bits);
        for i in 0..8 {
            t_enc |= ((user_bits & (0b1111 << i)) as u128) << (4 * i + 4);
        }

        // sync word
        t_enc |= 0b1011111111111100 << 64;

        let polarity_correction_bit: u128 = self.even_parity_bit(t_enc);
        t_enc |= polarity_correction_bit << polarity_idx;
        t_enc
    }

    fn generate_smpte_frame_buffer(&self, samples_per_bit: usize, frame_offset: i8) -> [f32; 2048] {
        let mut time_with_offs = self.state.ltc.clone();
        time_with_offs.f += frame_offset;
        time_with_offs.add_progress(0);
        let bits = self.generate_smpte_frame_bits(time_with_offs);

        let mut buf = [0f32; 2048];
        let mut current_parity = 1;
        for bit_idx in 0..80 {
            let frame_bit = (0x1 << bit_idx) & bits;
            for sample_idx in 0..samples_per_bit {
                let idx = sample_idx + bit_idx as usize * samples_per_bit;
                if sample_idx == 0 || (sample_idx == samples_per_bit / 2 && frame_bit != 0) {
                    current_parity *= -1;
                }

                buf[idx] = (current_parity as f32) * self.volume;
            }
        }

        let mut lp_buffer = [0_f32; 2048];
        self.low_pass(&buf, &mut lp_buffer);

        lp_buffer
    }

    fn increment(&mut self) {
        self.state.ltc.f += 1;
        self.state.ltc.add_progress(0);
    }

    fn decrement(&mut self) {
        self.state.ltc.f -= 1;
        self.state.ltc.add_progress(0);
    }

    fn preload_frame_buffer(&mut self) {
        let samples_per_frame: usize = self.samples_per_frame();
        let samples_per_bit: usize = self.samples_per_bit();

        let a_frame_buf =
            &self.generate_smpte_frame_buffer(samples_per_bit, 0)[..samples_per_frame];
        self.frame_buffer[..samples_per_frame].copy_from_slice(a_frame_buf);

        let b_frame_buf =
            &self.generate_smpte_frame_buffer(samples_per_bit, 1)[..samples_per_frame];
        self.frame_buffer[samples_per_frame..2 * samples_per_frame].copy_from_slice(b_frame_buf);

        //for (i, s) in self.frame_buffer.iter().enumerate() {
        //    println!("fbuf {i:03} {s}")
        //}

        //self.state.ltc.sub_us(1_000_000 / self.frame_rate() as u64);

        // DEBUG:
        //for (i, s) in self.frame_buffer.iter_mut().enumerate() {
        //*s = i as f32 / 8192.0
        //}
    }

    fn samples_per_bit(&self) -> usize {
        self.samples_per_frame() / 80
    }

    fn samples_per_frame(&self) -> usize {
        self.sample_rate / self.frame_rate() as usize
    }

    fn low_pass(&self, buf: &[f32], out: &mut [f32]) {
        //for (i, s) in buf.iter().enumerate() {
        //    println!("buf {i:03} {s}")
        //}
        const LP_WIDTH: usize = 3;
        let samples_per_frame = self.samples_per_frame();
        for idx in 0..samples_per_frame {
            let mut cumsum = 0.0;
            for offs_idx in idx..idx + LP_WIDTH {
                cumsum += if offs_idx < samples_per_frame {
                    buf[offs_idx]
                } else {
                    -buf[samples_per_frame - 10]
                }
            }
            cumsum /= LP_WIDTH as f32;
            out[idx] = cumsum;
        }
        //for (i, s) in out.iter().enumerate() {
        //    println!("out {i:03} {s}")
        //}
    }

    fn calculate_time_at_beat(&self, ctx: &AudioSourceContext, beat_idx: u16) -> TimecodeInstant {
        let mut time = TimecodeInstant {
            h: 0,
            m: 0,
            s: 0,
            f: 0,
            frame_progress: 0,
            frame_rate: self.frame_rate(),
        };
        let mut cursor = EventCursor::new(&ctx.cue.events);
        for i in 0..beat_idx {
            while cursor.at_or_before(beat_idx)
                && let Some(event) = cursor.get_next()
            {
                if let Some(EventDescription::TimecodeEvent {
                    time: new_time,
                    properties,
                }) = event.event
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
            .add_progress((samples * self.frame_rate() as usize * 65536 / sample_rate) as u16);
    }

    fn calculate_frame_overlap(&mut self, block_size: usize) -> u64 {
        // FIXME: will run slow(?) on some framerates where samples_per_bit gets truncated
        let samples_per_frame: usize = self.samples_per_frame();
        let samples_per_bit: usize = self.samples_per_bit();

        if self.subframe_sample > samples_per_frame as u64 {
            if self.state.running {
                self.increment();
            }

            self.subframe_sample -= samples_per_frame as u64;

            self.frame_buffer
                .copy_within(samples_per_frame..2 * samples_per_frame, 0);

            // write next frame into next frame buffer
            let next_frame_buf =
                &self.generate_smpte_frame_buffer(samples_per_bit, 1)[..samples_per_frame];
            self.frame_buffer[samples_per_frame..2 * samples_per_frame]
                .copy_from_slice(next_frame_buf);
        }

        let ret = self.subframe_sample;
        self.subframe_sample += block_size as u64;
        ret
    }

    fn audio_frame(&mut self, frame_size: usize) -> &[f32] {
        let subframe_sample = self.calculate_frame_overlap(frame_size);

        self.last_cycle_frame = self.state.ltc;

        //if self.state.running {
        //    // FIXME: handle drop-frame and color-framing options
        //    self.advance_by_samples(frame_size, self.sample_rate);
        //}

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
                ctx.cbnet
                    .notify(Message::Small(SmallMessage::TimecodeData(self.state)));
            }
            ControlAction::TransportStop => {
                self.state.running = false;
                ctx.cbnet
                    .notify(Message::Small(SmallMessage::TimecodeData(self.state)));
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
        if !self.state.running {
            return Ok(self.silence(ctx.frame_size));
        }

        ctx.cbnet
            .notify(Message::Small(SmallMessage::TimecodeData(self.state)));

        self.sample_rate = ctx.sample_rate;

        Ok(self.audio_frame(ctx.frame_size))
    }

    fn event_will_occur(&mut self, ctx: &AudioSourceContext, event: common::event::Event) {}

    fn event_occured(&mut self, ctx: &AudioSourceContext, event: common::event::Event) {
        if let Some(EventDescription::TimecodeEvent { time, properties }) = event.event {
            self.properties = properties;

            // FIXME: actually handle wall time
            if !self.properties.use_wall_time {
                self.state.ltc = time;
                self.preload_frame_buffer();
            }

            self.state.running = true;
        }

        if let Some(EventDescription::TimecodeStopEvent) = event.event {
            self.state.running = false;
            ctx.cbnet
                .notify(Message::Small(SmallMessage::TimecodeData(self.state)));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::audio::timecode::TimecodeSource;
    use common::mem::smpte::TimecodeProperties;
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
    #[ignore]
    fn smpte_ltc_spec() {
        // EBU Time-And-Control Code FOR TELEVISION TAPE-RECORDINGS
        // https://tech.ebu.ch/docs/tech/tech3097.pdf

        use super::*;

        let mut tc = TimecodeSource::init(48000, TimecodeProperties::default());
        tc.state.running = true;

        const FRAME_SIZE: usize = 256;
        const NUM_FRAMES: usize = 1000;
        let mut frame = [0_f32; FRAME_SIZE * NUM_FRAMES];
        for i in 0..NUM_FRAMES {
            let _ = &frame[i * FRAME_SIZE..(i + 1) * FRAME_SIZE]
                .copy_from_slice(tc.audio_frame(FRAME_SIZE));
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
        //assert!(
        //    (40.0..65.0).contains(&min),
        //    "Rise/fall time min is {}, should be >= 40",
        //    min
        //);
        assert!(
            (40.0..65.0).contains(&max),
            "Rise/fall time max is {}, should be <= 65",
            max
        );
    }

    #[test]
    fn start() {
        let mut tc = TimecodeSource::init(48000, TimecodeProperties::default());
        tc.state.running = true;
        assert_ne!(
            tc.audio_frame(256)
                .to_owned()
                .into_iter()
                .map(|v| v.abs())
                .sum::<f32>(),
            0.0
        )
    }

    #[test]
    fn smpte_ltc_eq() {
        use super::*;

        const BLOCK_SIZE: usize = 256;
        const NUM_BLOCKS: usize = 1000;
        const SAMPLE_RATE: usize = 48000;
        const SMPTE_FRAME_RATE: usize = 25;
        const SAMPLES_PER_FRAME: usize = SAMPLE_RATE / SMPTE_FRAME_RATE;
        const SAMPLES_PER_BIT: usize = SAMPLES_PER_FRAME / 80;

        let mut tc = TimecodeSource::init(
            SAMPLE_RATE,
            TimecodeProperties {
                user_bit_format: TimecodeUserBitFormat::DateTimezone,
                ..Default::default()
            },
        );

        let mut frame = vec![];
        for _ in 0..NUM_BLOCKS {
            frame.extend_from_slice(tc.audio_frame(BLOCK_SIZE));
        }

        //for (i, s) in frame.iter().enumerate().take(2060) {
        //    println!("frm {i:03} {:.0}", s * 8192.0)
        //}

        assert_ne!(frame.iter().sum::<f32>(), 0.0);

        let reader = hound::WavReader::open("tests/data/smpte_25fps.wav");

        assert!(reader.is_ok());
        let mut reader = reader.unwrap();
        let rsamples: Vec<i16> = reader.samples::<i16>().map(|o| o.unwrap_or(0)).collect();

        let rpeak = rsamples.iter().max().unwrap_or(&0);

        let cscale_factor = *rpeak as f32 / i16::MAX as f32;

        let csamples: Vec<i16> = frame
            .into_iter()
            .map(|v| (v * 32768.0 * cscale_factor * 2.0) as i16)
            .collect();

        const EQUAL_THRESHOLD: u16 = 2;
        for (i, (a, b)) in csamples.iter().zip(rsamples.iter()).enumerate() {
            if i % SAMPLES_PER_FRAME / SAMPLES_PER_BIT == 27
                || i % SAMPLES_PER_FRAME / SAMPLES_PER_BIT == 59
            {
                continue;
            }
            if a.abs().abs_diff(b.abs()) >= EQUAL_THRESHOLD {
                println!("Failed at sample {}", i);
                println!("splglb\tsplloc\tframe\tbit\tcks\trea");

                for j in i.saturating_sub(3)..i {
                    println!(
                        "{j}\t{}\t{}\t{}\t{}\t{}\tO",
                        j % SAMPLES_PER_FRAME,
                        j / SAMPLES_PER_FRAME,
                        j % SAMPLES_PER_FRAME / SAMPLES_PER_BIT,
                        csamples[j],
                        rsamples[j]
                    );
                }
                println!(
                    "{i}\t{}\t{}\t{}\t{}\t{}\t<--",
                    i % SAMPLES_PER_FRAME,
                    i / SAMPLES_PER_FRAME,
                    i % SAMPLES_PER_FRAME / SAMPLES_PER_BIT,
                    a,
                    b
                );
                for j in i + 1..i + 8 {
                    println!(
                        "{j}\t{}\t{}\t{}\t{}\t{}\t{}",
                        j % SAMPLES_PER_FRAME,
                        j / SAMPLES_PER_FRAME,
                        j % SAMPLES_PER_FRAME / SAMPLES_PER_BIT,
                        csamples[j],
                        rsamples[j],
                        if csamples[j].abs_diff(rsamples[j]) < EQUAL_THRESHOLD {
                            "O"
                        } else {
                            "X"
                        }
                    );
                }
                assert!(a.abs_diff(*b) < EQUAL_THRESHOLD);
            }
        }
    }

    #[test]
    #[ignore = "this test produces a file output"]
    fn export_ltc_as_wav() {
        const NUM_SECS: usize = 100;
        const SAMPLE_RATE: usize = 48000;
        const FRAME_SIZE: usize = 512;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut all_zeroes = true;

        let mut writer = hound::WavWriter::create("target/debug/ltc.wav", spec).unwrap();
        let mut tc = TimecodeSource::init(48000, TimecodeProperties::default());
        tc.state.running = true;
        for _ in (0..SAMPLE_RATE * NUM_SECS).step_by(FRAME_SIZE) {
            for sample in tc.audio_frame(FRAME_SIZE) {
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

        let mut tc = TimecodeSource::init(48000, TimecodeProperties::default());
        assert_eq!(tc.state.ltc, TimecodeInstant::new(25));
        tc.advance_by_samples(48000 / 50, 48000);
        tc.advance_by_samples(48000 / 50, 48000);

        assert_eq!(tc.state.ltc.s, 0);
        assert_eq!(tc.state.ltc.f, 1);
    }

    #[test]
    fn wraparound() {
        let mut time = TimecodeSource::init(48000, TimecodeProperties::default());
        for i in 0..20000 {
            time.state.running = true;
            time.audio_frame(256);
            println!("{}", time.state.ltc);
            assert_ne!(time.state.ltc.f, 25);
        }
    }
}
