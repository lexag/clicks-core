use common::{
    cue::{Cue, Show},
    event::EventCursor,
    local::{
        config::{LogContext, LogKind},
        status::CombinedStatus,
    },
    mem::typeflags::MessageType, protocol::{message::{LargeMessage, Message, SmallMessage}, request::ControlAction},
};
use jack::{AudioOut, Client, Control, Port, ProcessHandler, ProcessScope, Unowned};

use crate::{
    audio::source::{AudioSource, AudioSourceContext, SourceConfig}, logger::{self, LogItem}, CrossbeamNetwork
};

pub struct AudioProcessor {
    sources: Vec<SourceConfig>,
    cbnet: CrossbeamNetwork,
    status: CombinedStatus,
    ctx: AudioSourceContext,
    ports: (Vec<Port<AudioOut>>, Vec<Port<Unowned>>),
    status_changed_flag: bool,
}

impl AudioProcessor {
    pub fn new(
        sources: Vec<SourceConfig>,
        ports: (Vec<Port<AudioOut>>, Vec<Port<Unowned>>),
        cbnet: CrossbeamNetwork,
        show: Show,
    ) -> AudioProcessor {
        let mut a = AudioProcessor {
            ports,
            sources,
            cbnet,
            ctx: AudioSourceContext::default(),
            status: CombinedStatus::default(),
            status_changed_flag: false,
        };
        a.load_show(show);
        a.send_all_status();
        a
    }

    fn send_all_status(&self) {
        self.notify_push(MessageType::TransportData);
        self.notify_push(MessageType::BeatData);
        self.notify_push(MessageType::CueData);
        self.notify_push(MessageType::ShowData);
    }

    fn notify_push(&self, message_type: MessageType) {
        self.cbnet.notify(match message_type {
            MessageType::TransportData => Message::Small(SmallMessage::TransportData(self.status.transport)),
            MessageType::BeatData => Message::Small(SmallMessage::BeatData(self.status.beat_state())),
            MessageType::CueData => Message::Large(LargeMessage::CueData(self.status.cue.clone())),
            MessageType::ShowData => Message::Large(LargeMessage::ShowData(self.status.show.clone())),
            _ => {
                return;
            }
        });
    }

    fn load_cue(&mut self, cue: Cue) {
        self.status.transport.running = false;
        self.status.cue.cue = cue;

        self.cbnet.command(ControlAction::TransportStop);
        self.cbnet.command(ControlAction::TransportZero);
        self.notify_push(MessageType::CueData);
    }

    fn load_show(&mut self, show: Show) {
        self.status.show = show;
        self.cbnet.command(ControlAction::LoadCueByIndex(0));
        self.notify_push(MessageType::ShowData);
    }

    fn handle_command(&mut self, command: ControlAction) {
        self.cbnet.log(LogItem::new(
            format!("ControlAction: {command}"),
            LogContext::AudioProcessor,
            LogKind::Command,
        ));
        match command {
            ControlAction::DumpStatus => self.send_all_status(),
            ControlAction::TransportStart => {
                self.status.transport.running = true;
                self.notify_push(MessageType::TransportData);
            }
            ControlAction::TransportStop => {
                self.status.transport.running = false;
                self.notify_push(MessageType::TransportData);
            }

            ControlAction::TransportSeekBeat(..) | ControlAction::TransportJumpBeat(..) => {
                self.status_changed_flag = true;
            }

            ControlAction::LoadCueByIndex(idx) => {
                if idx < self.status.show.cues.len() as u8 {
                    self.load_cue(self.status.show.cues[idx as usize].clone());
                    self.status.cue.cue_idx = idx as u16;
                }
            }

            ControlAction::SetChannelGain(channel_idx, gain) => {
                self.sources[channel_idx as usize].set_gain(gain);
            }

            ControlAction::ChangeJumpMode(jumpmode) => {
                self.status.transport.vlt = jumpmode.vlt(self.status.transport.vlt);
                self.notify_push(MessageType::TransportData);
            }
            ControlAction::ChangePlayrate(playrate) => {
                self.status.transport.playrate_percent = playrate;
                self.notify_push(MessageType::TransportData);
            }

            _ => {}
        }

        // Pass on commands to all children
        // to do source specific implementations
        for source in &mut self.sources {
            source.source_device.command(&self.ctx, command);
        }

        self.compile_child_statuses();

        if command == ControlAction::TransportZero {
            self.notify_push(MessageType::BeatData);
            self.notify_push(MessageType::TransportData);
        }
    }

    fn compile_child_statuses(&mut self) {
        let current_beat = self.status.beat_state().beat_idx;
        for (source, status) in self.sources.iter_mut().zip(self.status.sources.iter_mut()) {
            *status = source.source_device.get_status(&self.ctx);
        }

        self.status.transport.vlt = self
            .status
            .beat_state()
            .requested_vlt_action
            .vlt(self.status.transport.vlt);

        self.status.transport.ltc = self.status.time_state();

        if self.status.beat_state().beat_idx != current_beat {
            self.send_beat_events_to_children(self.status.beat_state().beat_idx, false);
        }
    }

    // Get audio buffer from source[idx] and copy it to the JACK client output buffer.
    fn process_child(&mut self, idx: usize, ps: &ProcessScope) -> Control {
        let source = &mut self.sources[idx];
        let res = source.source_device.send_buffer(&self.ctx);
        if let Ok(buf) = res {
            let out_buf = self.ports.0[idx].as_mut_slice(ps);
            out_buf.clone_from_slice(buf);
            let gain = if self.status.transport.playrate_percent != 100 && idx != 0 {
                0.0
            } else {
                source.get_gain_mult()
            };
            for sample in out_buf {
                *sample *= gain;
            }
            Control::Continue
        } else {
            self.cbnet.log(LogItem::new(
                format!("Audio error occured in source {}.", idx),
                LogContext::AudioProcessor,
                LogKind::Error,
            ));
            Control::Quit
        }
    }

    fn update_context(&mut self, c: &Client, ps: &ProcessScope) {
        self.ctx = AudioSourceContext {
            jack_time: c.time(),
            frame_size: ps.n_frames() as usize,
            sample_rate: c.sample_rate(),
            beat: self.status.beat_state(),
            transport: self.status.transport,
            cbnet: self.cbnet.clone(),
            cue: self.status.cue.cue.clone(),
        }
    }

    fn send_beat_events_to_children(&mut self, beat_idx: u16, pre_event: bool) {
        let mut cursor = EventCursor::new(&self.status.cue.cue.events);
        cursor.seek(beat_idx);
        while cursor.at_or_before(beat_idx)
            && let Some(event) = cursor.get_next()
        {
            for source in &mut self.sources {
                if pre_event {
                    source.source_device.event_will_occur(&self.ctx, event);
                } else {
                    source.source_device.event_occured(&self.ctx, event);
                }
            }
        }
    }
}

impl ProcessHandler for AudioProcessor {
    fn process(&mut self, c: &Client, ps: &ProcessScope) -> Control {
        // Handle channel commands
        loop {
            match self.cbnet.cmd_rx.try_recv() {
                Ok(cmd) => self.handle_command(cmd),

                // If channel is empty, continue with process
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    break;
                }
                Err(err) => self.cbnet.log(LogItem::new(
                    format!("Error reading command: {}", err),
                    LogContext::AudioProcessor,
                    LogKind::Error,
                )),
            }
        }
        self.update_context(c, ps);
        // Get status from all sources and compile onto self.status
        self.compile_child_statuses();

        // If cue runs out: stop and go to next
        if self
            .status
            .cue
            .cue
            .get_beat(self.status.beat_state().beat_idx + 1)
            .is_none()
            && self.status.transport.running
            && self.status.beat_state().beat_idx < u16::MAX / 2
        {
            self.status.transport.running = false;
            self.cbnet.command(ControlAction::TransportStop);
            self.cbnet.command(ControlAction::LoadNextCue);
            self.cbnet.command(ControlAction::TransportZero);
        }

        // Warn of upcoming events
        if self.ctx.will_overrun_frame() {
            self.send_beat_events_to_children(self.status.beat_state().next_beat_idx, true);
        }

        self.update_context(c, ps);
        // Get audio frame buffers from all children and play in correct port
        for i in 0..self.sources.len() {
            if self.process_child(i, ps) == Control::Quit {
                return Control::Quit;
            };
        }

        if self.status.transport.running || self.status_changed_flag {
            self.notify_push(MessageType::TransportData);
            self.status_changed_flag = false;
        }

        Control::Continue
    }
}
