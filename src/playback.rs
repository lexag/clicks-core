use crate::audio::source::AudioSource;
use arc_swap::ArcSwap;
use common::{
    command::ControlCommand,
    cue::{BeatEvent, Cue},
    show::Show,
    status::AudioSourceStatus,
};
use jack::{AudioOut, Client, ClientOptions, Control, ProcessHandler, ProcessScope};
use std::{
    collections::HashMap, fs::File, num, path::PathBuf, str::FromStr, sync::Arc, thread::current,
};

const LOCAL_BUF_SIZE: usize = 48000;

type AudioBuffer = Vec<f32>;
struct AudioClip {
    pub clip_idx: Arc<ArcSwap<usize>>,
    buffer: Arc<ArcSwap<AudioBuffer>>,
    local_buffer: [f32; LOCAL_BUF_SIZE],
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
        self.local_buffer
            .copy_from_slice(&self.buffer.load()[start as usize..start as usize + LOCAL_BUF_SIZE]);
        &self.local_buffer[0..len]
    }
    pub fn read_index(&self) -> usize {
        **self.clip_idx.load()
    }
    pub fn get_length(&self) -> u32 {
        self.buffer.load().len() as u32
    }
}

pub struct PlaybackHandler {
    clips: HashMap<usize, Vec<AudioClip>>,
    config_path: PathBuf,
}

impl PlaybackHandler {
    fn new(config_path: PathBuf) -> PlaybackHandler {
        PlaybackHandler {
            config_path,
            clips: HashMap::new(),
        }
    }

    fn load_show(&mut self, show: Show, num_channels: usize) {
        let mut max_clips_per_cue_per_channel: HashMap<usize, usize> = HashMap::new();
        for (cue_idx, cue) in show.cues.iter().enumerate() {
            let mut clips_in_cue = 0;
            for beat in cue.get_beats() {
                for event in beat.events {
                    match event {
                        BeatEvent::PlaybackEvent {
                            channel_idx,
                            clip_idx,
                            sample,
                        } => {
                            clips_in_cue += 1;
                        }
                        _ => {}
                    }
                }
            }
            max_clips_per_cue_per_channel.insert(
                cue_idx,
                usize::max(
                    clips_in_cue,
                    *(max_clips_per_cue_per_channel.get(&cue_idx).unwrap()),
                ),
            );
        }

        for channel_idx in 0..num_channels {
            let max_clips_in_channel: usize =
                *(max_clips_per_cue_per_channel.get(&channel_idx).unwrap());
            for clip_idx in 0..max_clips_in_channel {
                self.clips
                    .get_mut(&channel_idx)
                    .unwrap()
                    .push(AudioClip::new());
            }
        }
    }

    fn create_audio_sources(&mut self) -> Vec<PlaybackDevice> {
        let mut devices = vec![];
        for (channel_idx, clips) in &self.clips {
            let mut device = PlaybackDevice::new(*channel_idx, self.config_path.clone());
            for clip in clips {
                device.clips.push(AudioClip {
                    clip_idx: Arc::clone(&clip.clip_idx),
                    buffer: Arc::clone(&clip.buffer),
                    local_buffer: [0.0f32; LOCAL_BUF_SIZE],
                });
            }
            devices.push(device);
        }
        return devices;
    }

    fn load_cue(&self, cue: Cue) {
        let mut clips_per_cue: HashMap<usize, Vec<usize>> = HashMap::new();
        for beat in cue.get_beats() {
            for event in beat.events {
                match event {
                    BeatEvent::PlaybackEvent {
                        channel_idx,
                        clip_idx,
                        sample,
                    } => clips_per_cue.get_mut(&channel_idx).unwrap().push(clip_idx),
                    _ => {}
                }
            }
        }
        for (channel_idx, clips) in &self.clips {
            let mut clips_in_cue = clips_per_cue.get(&channel_idx).unwrap().clone();
            clips_in_cue.sort();
            clips_in_cue.dedup();
            for (incue_index, clip_idx) in clips_in_cue.iter().enumerate() {
                let mut reader = hound::WavReader::open(
                    self.config_path
                        .join(format!("{:0>3}/{:0>3}.wav", channel_idx, clip_idx)),
                )
                .unwrap();
                let buf = reader.samples::<f32>().map(|s| s.unwrap()).collect();

                clips[incue_index].write(*clip_idx, buf);
            }
        }
    }
}

pub struct PlaybackDevice {
    pub channel_idx: usize,
    pub current_sample: i32,
    pub current_clip: usize,
    clips: Vec<AudioClip>,
    config_path: PathBuf,
    cue: Cue,
    active: bool,
}

impl PlaybackDevice {
    fn new(channel_idx: usize, config_path: PathBuf) -> PlaybackDevice {
        PlaybackDevice {
            channel_idx,
            current_sample: 0,
            current_clip: 0,
            clips: vec![],
            config_path,
            cue: Cue::empty(),
            active: false,
        }
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
                        let samples_to_next_beat =
                            status.us_to_next_beat * _c.sample_rate() / 1000000;
                        if (samples_to_next_beat as u32) < num_samples {
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

            if self.current_sample < 0
                || self.current_sample as u32 + num_samples
                    > self.clips[self.current_clip].get_length()
            {
                return Ok(&[0.0f32; 96000][0..num_samples as usize]);
            }
            let buf = self.clips[self.current_clip]
                .read_buffer_slice(self.current_sample as u32, num_samples as usize);
            self.current_sample += num_samples as i32;
            return Ok(&buf[0..num_samples as usize]);
        } else {
            return Ok(&[0.0f32; 96000][0..num_samples as usize]);
        }
    }
    fn command(
        &mut self,
        command: common::command::ControlCommand,
    ) -> Result<(), common::command::CommandError> {
        Ok(())
    }

    fn get_status(
        &mut self,
        _c: &jack::Client,
        _ps: &jack::ProcessScope,
    ) -> common::status::AudioSourceStatus {
        AudioSourceStatus::None
    }
}
