use crate::{
    audio::source::{AudioSource, AudioSourceContext, SourceConfig},
    logger,
};
use arc_swap::ArcSwap;
use common::{
    cue::{Cue, Show},
    event::{EventCursor, EventDescription},
    local::{
        config::{LogContext, LogKind},
        status::{AudioSourceState, PlaybackState},
    },
    protocol::request::ControlAction,
};
use std::{fmt::Debug, ops::Div, path::PathBuf, sync::Arc};

const LOCAL_BUF_SIZE: usize = 48000;

type AudioBuffer = Vec<f32>;
struct AudioClip {
    pub clip_idx: Arc<ArcSwap<usize>>,
    buffer: Arc<ArcSwap<AudioBuffer>>,
    local_buffer: [f32; LOCAL_BUF_SIZE],
}

impl Debug for AudioClip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "AudioClip: clip_idx: {}", self.clip_idx)
    }
}

impl AudioClip {
    pub fn new() -> Self {
        Self {
            clip_idx: Arc::new(ArcSwap::from_pointee(0)),
            buffer: Arc::new(ArcSwap::from_pointee(vec![])),
            local_buffer: [0.0f32; LOCAL_BUF_SIZE],
        }
    }

    // Called in non-RT thread
    pub fn write(&self, idx: usize, buffer: Vec<f32>) {
        self.clip_idx.store(Arc::new(idx));
        self.buffer.store(Arc::new(buffer));
    }

    // Called in RT thread
    pub fn read_buffer_slice(&mut self, start: u32, len: usize) -> &[f32] {
        let buf = &self.buffer.load();
        self.local_buffer[..len].copy_from_slice(&buf[start as usize..start as usize + len]);
        return &self.local_buffer[0..len];
    }
    pub fn read_index(&self) -> usize {
        **self.clip_idx.load()
    }
    pub fn get_length(&self) -> u32 {
        self.buffer.load().len() as u32
    }
}

pub struct PlaybackHandler {
    clips: Vec<Vec<AudioClip>>,
    show_path: PathBuf,
    num_channels: usize,
}

impl PlaybackHandler {
    pub fn new(show_path: PathBuf, num_channels: usize) -> PlaybackHandler {
        PlaybackHandler {
            show_path,
            clips: Vec::new(),
            num_channels,
        }
    }

    fn num_channel_clips_in_cue(&self, cue: &Cue, channel: usize) -> usize {
        if channel > self.num_channels {
            return 0;
        }
        let mut clips_in_cue = 0;
        let mut cursor = EventCursor::new(&cue.events);
        while let Some(event) = cursor.get_next() {
            if let Some(EventDescription::PlaybackEvent {
                channel_idx,
                clip_idx: _,
                sample: _,
            }) = event.event
                && channel_idx == channel as u16
            {
                clips_in_cue += 1;
            }
        }

        clips_in_cue
    }

    fn load_wav_buf(&self, channel: usize, clip: usize) -> Vec<f32> {
        let mut reader = match hound::WavReader::open(
            self.show_path
                .join(format!("playback_media/{:0>3}/{:0>3}.wav", channel, clip)),
        ) {
            Ok(val) => val,
            Err(err) => {
                logger::log(
                    format!("Error opening playback media: {}", err),
                    LogContext::AudioSource,
                    LogKind::Error,
                );
                return vec![0.0; 48000];
            }
        };
        let buf: Vec<f32> = match reader.spec().sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .map(|sample| {
                    if let Err(err) = sample {
                        logger::log(
                            format!("Error opening playback media: {}", err),
                            LogContext::AudioSource,
                            LogKind::Error,
                        );
                        return 0.0;
                    }
                    sample.expect("Err already handled.")
                })
                .collect(),
            hound::SampleFormat::Int => reader
                .samples::<i32>()
                .map(|sample| {
                    if let Err(err) = sample {
                        logger::log(
                            format!("Error opening playback media: {}", err),
                            LogContext::AudioSource,
                            LogKind::Error,
                        );
                        return 0.0;
                    }
                    (sample.expect("Err already handled.") as f32).div(32768.0)
                })
                .collect(),
        };
        buf
    }

    // Returns a vector indexed by channel where each element is that channel's list of clip idxs
    // in this cue. The inner list is unordered (ordered by first apperance in the cue) and
    // non-duplicated.
    fn clip_idxs_in_cue(&self, cue: &Cue) -> Vec<Vec<usize>> {
        let mut clips: Vec<Vec<usize>> = Vec::new();
        for _ in 0..self.num_channels {
            clips.push(Vec::new());
        }
        let mut cursor = EventCursor::new(&cue.events);
        while let Some(event) = cursor.get_next() {
            if let Some(EventDescription::PlaybackEvent {
                channel_idx,
                clip_idx,
                sample: _,
            }) = event.event
                && !clips[channel_idx as usize].contains(&(clip_idx as usize))
            {
                clips[channel_idx as usize].push(clip_idx as usize);
            }
        }

        clips
    }

    pub fn load_show(&mut self, show: Show) {
        let max_clips: Vec<usize> = (0..self.num_channels)
            .clone()
            .map(|channel| {
                show.cues
                    .iter()
                    .map(|cue| self.num_channel_clips_in_cue(cue, channel))
                    .max()
                    .unwrap_or(0)
            })
            .collect();

        self.clips.clear();
        for (channel, mc_channel) in max_clips.iter().enumerate().take(self.num_channels) {
            self.clips.push(Vec::new());
            for _ in 0..*mc_channel {
                self.clips[channel].push(AudioClip::new());
            }
        }
    }

    pub fn create_audio_sources(&mut self) -> Vec<SourceConfig> {
        let mut devices = vec![];
        for channel in 0..self.num_channels {
            let mut device = PlaybackDevice::new(channel as u16, self.show_path.clone());
            for clip in &self.clips[channel] {
                device.clips.push(AudioClip {
                    clip_idx: Arc::clone(&clip.clip_idx),
                    buffer: Arc::clone(&clip.buffer),
                    local_buffer: [0.0f32; LOCAL_BUF_SIZE],
                });
            }
            devices.push(SourceConfig::new(
                format!("playback_{channel}"),
                Box::new(device),
            ));
        }
        devices
    }

    pub fn load_cue(&self, cue: Cue) {
        for (channel, clips) in self.clip_idxs_in_cue(&cue).iter_mut().enumerate() {
            clips.sort();
            for (i, clip) in clips.iter().enumerate() {
                let buf = self.load_wav_buf(channel, *clip);
                self.clips[channel][i].write(*clip, buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use common::event::Event;

    use super::*;
    #[test]

    fn clips_counter() {
        let pbh = PlaybackHandler::new(PathBuf::new(), 32);
        for length in [0, 1, 2, 56, 100] {
            for channel in (0..34).step_by(2) {
                let mut cue = Cue::empty();
                for i in 0..length {
                    cue.events.set(
                        i.min(63),
                        Event::new(
                            i as u16 * 2,
                            EventDescription::PlaybackEvent {
                                channel_idx: channel,
                                clip_idx: 0,
                                sample: 0,
                            },
                        ),
                    );
                }
                if channel <= 32 {
                    assert_eq!(
                        pbh.num_channel_clips_in_cue(&cue, channel as usize),
                        length.min(64) as usize
                    );
                } else {
                    assert_eq!(pbh.num_channel_clips_in_cue(&cue, channel as usize), 0);
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct PlaybackDevice {
    pub channel_idx: u16,
    pub current_sample: i32,
    pub current_clip: usize,
    clips: Vec<AudioClip>,
    show_path: PathBuf,
    active: bool,
}

impl PlaybackDevice {
    fn new(channel_idx: u16, show_path: PathBuf) -> PlaybackDevice {
        PlaybackDevice {
            channel_idx,
            current_sample: 0,
            current_clip: 0,
            clips: vec![],
            show_path,
            active: false,
        }
    }

    fn calculate_time_at_beat(
        &mut self,
        ctx: &AudioSourceContext,
        beat_idx: u16,
    ) -> (usize, bool, i32) {
        let mut running_active = false;
        let mut running_clip = 0;
        let mut running_sample = 0_i64;
        let mut time_off_us = 0_u64;
        let mut cursor = EventCursor::new(&ctx.cue.events);
        for i in 0..beat_idx {
            while cursor.at_or_before(i)
                && let Some(event) = cursor.get_next()
            {
                match event.event {
                    Some(EventDescription::PlaybackEvent {
                        channel_idx,
                        clip_idx,
                        sample,
                    }) => {
                        if channel_idx == self.channel_idx {
                            running_sample = sample as i64;
                            running_clip = clip_idx;
                            running_active = true;
                            time_off_us = 0;
                        }
                    }
                    Some(EventDescription::PlaybackStopEvent { channel_idx }) => {
                        if channel_idx == self.channel_idx {
                            running_active = false;
                        }
                    }
                    _ => {}
                }
            }
            time_off_us += ctx.cue.get_beat(i).unwrap_or_default().length as u64;
        }
        // TODO: support multiple and resampled sample rates
        running_sample += time_off_us as i32 * 48 / 1000;
        (running_clip as usize, running_active, running_sample)
    }
}

impl AudioSource for PlaybackDevice {
    fn send_buffer(&mut self, ctx: &AudioSourceContext) -> Result<&[f32], jack::Error> {
        if !ctx.transport.running {
            return Ok(self.silence(ctx.frame_size));
        }

        // If currently not playing or prerolling before playing, return silence
        if !self.active || self.current_sample < 0 {
            return Ok(self.silence(ctx.frame_size));
        }

        // If about to run out of clip length, return silence and stop playback
        if self.current_sample + ctx.frame_size as i32
            > self.clips[self.current_clip].get_length() as i32
        {
            self.active = false;
            return Ok(self.silence(ctx.frame_size));
        }

        // All is well, return clip audio
        let buf = self.clips[self.current_clip]
            .read_buffer_slice(self.current_sample as u32, ctx.frame_size);
        self.current_sample += ctx.frame_size as i32;
        Ok(&buf[0..ctx.frame_size])
    }

    fn command(&mut self, ctx: &AudioSourceContext, command: ControlAction) {
        match command {
            ControlAction::TransportStop => {
                self.active = false;
            }
            ControlAction::TransportZero => {
                self.active = false;
            }

            ControlAction::TransportJumpBeat(beat_idx) => {
                (self.current_clip, self.active, self.current_sample) =
                    self.calculate_time_at_beat(ctx, beat_idx as u16);
            }
            ControlAction::TransportSeekBeat(beat_idx) => {
                (self.current_clip, self.active, self.current_sample) =
                    self.calculate_time_at_beat(ctx, beat_idx as u16);
                // TODO: Support multiple and mixed sample rates
                self.current_sample -= (ctx.transport.us_to_next_beat as i32) * 48 / 1000
            }
            _ => {}
        }
    }

    fn get_status(&mut self, ctx: &AudioSourceContext) -> AudioSourceState {
        let mut clips = [0u16; 16];
        for (i, clip) in clips.iter_mut().enumerate() {
            *clip = self.clips[i].read_index() as u16;
        }
        AudioSourceState::PlaybackStatus(PlaybackState {
            clips,
            clip_idx: self.current_clip as u16,
            current_sample: self.current_sample,
            playing: self.active,
        })
    }

    fn event_will_occur(&mut self, ctx: &AudioSourceContext, event: common::event::Event) {
        match event.event {
            Some(EventDescription::PlaybackEvent {
                channel_idx,
                clip_idx,
                sample,
            }) => {
                if channel_idx != self.channel_idx || !ctx.will_overrun_frame() {
                    return;
                }
                // if this cycle will run over the edge into next beat, we start playback
                // slightly before start of audio clip, so it aligns on the downbeat
                // sample.
                self.active = true;
                self.current_sample = sample;
                for (i, clip) in self.clips.iter().enumerate() {
                    if clip.read_index() == clip_idx as usize {
                        self.current_clip = i;
                    } else {
                        self.active = false;
                    }
                }
            }
            Some(EventDescription::PlaybackStopEvent { channel_idx }) => {
                if channel_idx != self.channel_idx {
                    return;
                }
                self.active = false;
            }
            _ => {}
        }
    }

    fn event_occured(&mut self, ctx: &AudioSourceContext, event: common::event::Event) {}
}
