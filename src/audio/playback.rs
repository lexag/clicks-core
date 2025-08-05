use crate::{
    audio::source::{AudioSource, SourceConfig},
    logger,
};
use arc_swap::ArcSwap;
use common::{
    command::ControlCommand,
    cue::{BeatEvent, Cue},
    show::Show,
    status::{AudioSourceStatus, ProcessStatus},
};
use jack::{AudioOut, Client, ClientOptions, Control, ProcessHandler, ProcessScope};
use std::{
    collections::HashMap, fmt::Debug, fs::File, num, ops::Div, path::PathBuf, str::FromStr,
    sync::Arc, thread::current,
};

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
        for beat in cue.get_beats() {
            for event in beat.events {
                match event {
                    BeatEvent::PlaybackEvent {
                        channel_idx,
                        clip_idx,
                        sample,
                    } => {
                        if channel_idx == channel {
                            clips_in_cue += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        clips_in_cue
    }

    fn load_wav_buf(&self, channel: usize, clip: usize) -> Vec<f32> {
        let mut reader = hound::WavReader::open(
            self.show_path
                .join(format!("playback_media/{:0>3}/{:0>3}.wav", channel, clip)),
        )
        .unwrap();
        let buf: Vec<f32> = match reader.spec().sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .map(|sample| {
                    if let Err(err) = sample {
                        logger::log(
                            format!("Error opening playback media: {}", err),
                            logger::LogContext::AudioSource,
                            logger::LogKind::Error,
                        );
                        return 0.0;
                    }
                    return sample.unwrap();
                })
                .collect(),
            hound::SampleFormat::Int => reader
                .samples::<i32>()
                .map(|sample| {
                    if let Err(err) = sample {
                        logger::log(
                            format!("Error opening playback media: {}", err),
                            logger::LogContext::AudioSource,
                            logger::LogKind::Error,
                        );
                        return 0.0;
                    }
                    return (sample.unwrap() as f32).div(32768.0);
                })
                .collect(),
        };
        return buf;
    }

    // Returns a vector indexed by channel where each element is that channel's list of clip idxs
    // in this cue. The inner list is unordered (ordered by first apperance in the cue) and
    // non-duplicated.
    fn clip_idxs_in_cue(&self, cue: &Cue) -> Vec<Vec<usize>> {
        let mut clips: Vec<Vec<usize>> = Vec::new();
        for i in 0..self.num_channels {
            clips.push(Vec::new());
        }
        for beat in cue.get_beats() {
            for event in beat.events {
                match event {
                    BeatEvent::PlaybackEvent {
                        channel_idx,
                        clip_idx,
                        sample,
                    } => {
                        if !clips[channel_idx].contains(&clip_idx) {
                            clips[channel_idx].push(clip_idx);
                        }
                    }
                    _ => {}
                };
            }
        }
        return clips;
    }

    pub fn load_show(&mut self, show: Show) {
        let mut needed_clip_slots_per_cue: HashMap<usize, usize> = HashMap::new();

        // The plan:
        // Per channel, find max clips per cue and make that many clips in that channel slot

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
        for channel in 0..self.num_channels {
            self.clips.push(Vec::new());
            for clip_idx in 0..max_clips[channel] {
                self.clips[channel].push(AudioClip::new());
            }
        }
    }

    pub fn create_audio_sources(&mut self) -> Vec<SourceConfig> {
        let mut devices = vec![];
        for channel in 0..self.num_channels {
            let mut device = PlaybackDevice::new(channel, self.show_path.clone());
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
        return devices;
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
    use super::*;
    #[test]

    fn clips_counter() {
        let pbh = PlaybackHandler::new(PathBuf::new(), 32);
        for length in [0, 1, 2, 100, 10000] {
            for channel in (0..34) {
                let cue = Cue {
                    metadata: common::cue::CueMetadata {
                        name: String::new(),
                        human_ident: String::new(),
                    },
                    beats: (0..length)
                        .map(|i| common::cue::Beat {
                            events: vec![BeatEvent::PlaybackEvent {
                                channel_idx: channel,
                                clip_idx: 0,
                                sample: 0,
                            }],
                            ..Default::default()
                        })
                        .collect(),
                };
                if channel <= 32 {
                    assert_eq!(pbh.num_channel_clips_in_cue(&cue, channel), length);
                } else {
                    assert_eq!(pbh.num_channel_clips_in_cue(&cue, channel), 0);
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct PlaybackDevice {
    pub channel_idx: usize,
    pub current_sample: i32,
    pub current_clip: usize,
    clips: Vec<AudioClip>,
    show_path: PathBuf,
    cue: Cue,
    active: bool,
    status: ProcessStatus,
}

impl PlaybackDevice {
    fn new(channel_idx: usize, show_path: PathBuf) -> PlaybackDevice {
        PlaybackDevice {
            status: ProcessStatus::default(),
            channel_idx,
            current_sample: 0,
            current_clip: 0,
            clips: vec![],
            show_path,
            cue: Cue::empty(),
            active: false,
        }
    }

    fn calculate_time_at_beat(&mut self, beat_idx: usize) -> (usize, bool, i32) {
        let mut running_active = false;
        let mut running_clip = 0;
        let mut running_sample = 0;
        let mut time_off_us = 0_u64;
        for i in 0..beat_idx {
            for event in self.cue.get_beat(i).unwrap_or_default().events {
                match event {
                    BeatEvent::PlaybackEvent {
                        channel_idx,
                        clip_idx,
                        sample,
                    } => {
                        if channel_idx == self.channel_idx {
                            running_sample = sample;
                            running_clip = clip_idx;
                            running_active = true;
                            time_off_us = 0;
                        }
                    }
                    BeatEvent::PlaybackStopEvent { channel_idx } => {
                        if channel_idx == self.channel_idx {
                            running_active = false;
                        }
                    }
                    _ => {}
                }
            }
            time_off_us += self.cue.get_beat(i).unwrap().length as u64;
        }
        // TODO: support multiple and resampled sample rates
        running_sample += time_off_us as i32 * 48 / 1000;
        return (running_clip, running_active, running_sample);
    }
}

impl AudioSource for PlaybackDevice {
    fn send_buffer(
        &mut self,
        _c: &jack::Client,
        _ps: &jack::ProcessScope,
        status: common::status::ProcessStatus,
    ) -> Result<&[f32], jack::Error> {
        let num_samples = _ps.n_frames();
        //println!("{:?}", self);
        //println!("status: {:?}", status);
        if status.running {
            for event in self
                .cue
                .get_beat(status.next_beat_idx)
                .unwrap_or_default()
                .events
            {
                match event {
                    BeatEvent::PlaybackEvent {
                        channel_idx,
                        clip_idx,
                        sample,
                    } => {
                        if channel_idx != self.channel_idx {
                            continue;
                        }
                        // if this cycle will run over the edge into next beat, we start playback
                        // slightly before start of audio clip, so it aligns on the downbeat
                        // sample.
                        let samples_to_next_beat: u32 = (status.us_to_next_beat / 10) as u32
                            * (_c.sample_rate() / 100) as u32
                            / 1000;
                        if samples_to_next_beat < num_samples {
                            self.active = true;
                            self.current_sample = sample;
                            for (i, clip) in self.clips.iter().enumerate() {
                                if clip.read_index() == clip_idx {
                                    self.current_clip = i;
                                } else {
                                    self.active = false;
                                }
                            }
                        }
                    }
                    BeatEvent::PlaybackStopEvent { channel_idx } => {
                        if channel_idx != self.channel_idx {
                            continue;
                        }
                        self.active = false;
                    }
                    _ => {}
                }
            }

            if self.active {
                if self.current_sample < 0 {
                    return Ok(&[0.0f32; 96000][0..num_samples as usize]);
                }
                if self.current_sample as u32 + num_samples
                    > self.clips[self.current_clip].get_length()
                {
                    self.active = false;
                    return Ok(&[0.0f32; 96000][0..num_samples as usize]);
                }
                let buf = self.clips[self.current_clip]
                    .read_buffer_slice(self.current_sample as u32, num_samples as usize);
                self.current_sample += num_samples as i32;
                return Ok(&buf[0..num_samples as usize]);
            }
        }
        return Ok(&[0.0f32; 96000][0..num_samples as usize]);
    }
    fn command(
        &mut self,
        command: common::command::ControlCommand,
    ) -> Result<(), common::command::CommandError> {
        match command {
            ControlCommand::LoadCue(cue) => {
                self.cue = cue;
            }
            ControlCommand::TransportStop => {
                self.active = false;
            }
            ControlCommand::TransportZero => {
                self.active = false;
            }

            ControlCommand::TransportJumpBeat(beat_idx) => {
                (self.current_clip, self.active, self.current_sample) =
                    self.calculate_time_at_beat(beat_idx);
            }
            ControlCommand::TransportSeekBeat(beat_idx) => {
                (self.current_clip, self.active, self.current_sample) =
                    self.calculate_time_at_beat(beat_idx);
                // TODO: Support multiple and mixed sample rates
                self.current_sample -= (self.status.us_to_next_beat as i32) * 48 / 1000
            }
            _ => {}
        }
        Ok(())
    }

    fn get_status(
        &mut self,
        _c: &jack::Client,
        _ps: &jack::ProcessScope,
    ) -> common::status::AudioSourceStatus {
        AudioSourceStatus::PlaybackStatus(common::status::PlaybackStatus {
            clips: self.clips.iter().map(|c| c.read_index()).collect(),
            clip_idx: self.current_clip,
            current_sample: self.current_sample,
            playing: self.active,
        })
    }
}
