use std::sync::Weak;

use crate::{
    audio::{
        config::AudioConfig, notification::JACKNotificationHandler, processor::AudioProcessor,
        source::SourceConfig,
    },
    logger, CrossbeamNetwork,
};
use common::{
    config::AudioConfiguration,
    network::{JACKStatus, StatusMessageKind},
};
use crossbeam_channel::{Receiver, Sender};
use jack::{
    AsyncClient, AudioIn, AudioOut, Client, ClientOptions, ClientStatus, Error, Port, PortFlags,
    Unowned,
};

use common::command::ControlCommand;

pub struct AudioHandler {
    pub client: Option<AsyncClient<JACKNotificationHandler, AudioProcessor>>,
    pub num_sources: usize,
    config: AudioConfiguration,
    jack_server_process: Option<std::process::Child>,
    cbnet: CrossbeamNetwork,
}

impl AudioHandler {
    pub fn new(num_sources: usize, cbnet: CrossbeamNetwork) -> AudioHandler {
        AudioHandler {
            cbnet,
            client: None,
            num_sources,
            config: AudioConfiguration::default(),
            jack_server_process: None,
        }
    }

    pub fn configure(&mut self, config: AudioConfiguration) {
        self.config = config
    }

    pub fn start(&mut self, sources: Vec<SourceConfig>) {
        self.start_server();
        std::thread::sleep(std::time::Duration::from_secs(5));
        let client = self.start_client();
        let mut ports: (Vec<Port<AudioOut>>, Vec<Port<Unowned>>) = (vec![], vec![]);
        ports.0 = self.init_client_ports(&client);
        ports.1 = self.collect_system_ports(&client);

        let processor = AudioProcessor::new(sources, ports, self.cbnet.clone());
        let ac = client
            .activate_async(JACKNotificationHandler, processor)
            .unwrap();
        self.client = Some(ac);
    }

    fn get_ports(&self) -> (Vec<Port<Unowned>>, Vec<Port<Unowned>>) {
        let mut ports: (Vec<Port<Unowned>>, Vec<Port<Unowned>>) = (vec![], vec![]);
        let client = self.client.as_ref().unwrap().as_client();
        let mut i_ports = client.ports(
            Some(&self.config.client.name),
            Some("32 bit float mono audio"),
            PortFlags::IS_OUTPUT,
        );
        i_ports.sort_by_key(|name| {
            let mut new_name = name.clone();
            new_name.retain(|c| c.is_numeric());
            return new_name.parse::<usize>().unwrap_or_default();
        });
        ports.0 = i_ports
            .iter()
            .map(|name| client.port_by_name(name).unwrap())
            .collect();

        let mut o_ports = client.ports(
            Some(&self.config.server.system_name),
            Some("32 bit float mono audio"),
            PortFlags::IS_INPUT,
        );
        o_ports.sort_by_key(|name| {
            let mut new_name = name.clone();
            new_name.retain(|c| c.is_numeric());
            return new_name.parse::<usize>().unwrap_or_default();
        });
        ports.1 = o_ports
            .iter()
            .map(|name| client.port_by_name(name).unwrap())
            .collect();

        return ports;
    }

    pub fn try_route_ports(&mut self, from: usize, to: usize, connect: bool) -> bool {
        let ports = self.get_ports();
        let p_from = ports.0[from].clone();
        let p_to = ports.1[to].clone();
        let res = if connect {
            self.client
                .as_mut()
                .unwrap()
                .as_client()
                .connect_ports(&p_from, &p_to)
        } else {
            self.client
                .as_mut()
                .unwrap()
                .as_client()
                .disconnect_ports(&p_from, &p_to)
        };

        if let Ok(_) = res {
            logger::log(
                format!("Set port [{from}] -> [{to}] to {connect}"),
                logger::LogContext::AudioHandler,
                logger::LogKind::Note,
            );

            return true;
        }

        match res.unwrap_err() {
            Error::PortConnectionError {
                source,
                destination,
                code_or_message,
            } => {
                logger::log(format!("JACK Connection Error occured attempting to connect [{source}] to [{destination}]. {code_or_message}"), logger::LogContext::AudioHandler, logger::LogKind::Error);
            }
            Error::PortAlreadyConnected(source, destination) => {
                logger::log(format!(
                        "JACK Connection Error occured attempting to connect [{source}] to [{destination}]. Ports are already connected."
                    ), logger::LogContext::AudioHandler, logger::LogKind::Error);
            }
            Error::PortDisconnectionError => {
                logger::log(
                    format!("JACK Disconnection Error occured attempting to connect port #{from} to #{to}."),
                    logger::LogContext::AudioHandler,
                    logger::LogKind::Error,
                );
            }
            _ => {
                logger::log(
                    format!("Unhandled JACK error connecting [{from}] to [{to}]"),
                    logger::LogContext::AudioHandler,
                    logger::LogKind::Error,
                );
            }
        }
        return false;
    }

    pub fn get_jack_status(&self) -> Option<JACKStatus> {
        if self.client.is_none() {
            return None;
        }
        let ports = self.get_ports();
        return Some(JACKStatus {
            io_size: (self.num_sources, self.config.server.num_channels),
            buffer_size: self.client.as_ref().unwrap().as_client().buffer_size() as usize,
            sample_rate: self.client.as_ref().unwrap().as_client().sample_rate(),
            frame_size: 0,
            connections: ports
                .0
                .iter()
                .enumerate()
                .map(|(a_idx, a_port)| {
                    ports
                        .1
                        .iter()
                        .enumerate()
                        .map(|(b_idx, b_port)| {
                            if a_port
                                .is_connected_to(&b_port.name().unwrap())
                                .unwrap_or_default()
                            {
                                (a_idx, b_idx)
                            } else {
                                (usize::MAX, usize::MAX)
                            }
                        })
                        .collect::<Vec<(usize, usize)>>()
                })
                .flatten()
                .filter(|(a, b)| *a < usize::MAX && *b < usize::MAX)
                .collect(),
            client_name: self.config.client.name.clone(),
            output_name: self.config.server.system_name.clone(),
        });
    }

    pub fn shutdown(&mut self) {
        if self.jack_server_process.is_none() {
            return;
        }
        &self.jack_server_process.as_mut().unwrap().kill();
    }

    pub fn start_server(&mut self) {
        self.jack_server_process = Some(
            std::process::Command::new("jackd")
                .arg("-R")
                .args(["-d", "alsa"])
                .args(["-d", &self.config.server.device_name])
                .args(["-r", &self.config.server.sample_rate.to_string()])
                .spawn()
                .unwrap(),
        );
    }

    pub fn start_client(&mut self) -> Client {
        let client_res = Client::new(
            &self.config.client.name.to_string(),
            ClientOptions::NO_START_SERVER,
        );
        match client_res {
            Err(err) => {
                panic!("Couldn't start JACK client: {err:?}");
            }
            Ok((client, status)) => {
                logger::log(
                    format!("Opened JACK client ({status:?})"),
                    logger::LogContext::AudioHandler,
                    logger::LogKind::Note,
                );
                client
            }
        }
    }

    pub fn init_client_ports(&self, client: &Client) -> Vec<Port<AudioOut>> {
        let mut ports = vec![];
        // Register io_matrix.0 amount of ports on the client and save for processor
        // reference
        for c_out_idx in 0..self.num_sources {
            ports.push(
                client
                    .register_port(&c_out_idx.to_string(), AudioOut::default())
                    .expect("Port register failed"),
            );
        }
        return ports;
    }

    pub fn collect_system_ports(&self, client: &Client) -> Vec<Port<Unowned>> {
        let client = self.client.as_ref().unwrap().as_client();
        let mut ports = client.ports(
            Some(&self.config.server.system_name),
            Some("32 bit float mono audio"),
            PortFlags::IS_INPUT,
        );
        ports.sort_by_key(|name| {
            let mut new_name = name.clone();
            new_name.retain(|c| c.is_numeric());
            return new_name.parse::<usize>().unwrap_or_default();
        });
        logger::log(
            format!("Found {} system ports.", ports.len()),
            logger::LogContext::AudioHandler,
            logger::LogKind::Note,
        );
        return ports
            .iter()
            .map(|name| client.port_by_name(name).unwrap())
            .collect();
    }

    pub fn send_status(&self) {
        let jack_status = JACKStatus {
            io_size: (self.num_sources, self.config.server.num_channels),
            sample_rate: self.client.as_ref().unwrap().as_client().sample_rate(),
            buffer_size: self.client.as_ref().unwrap().as_client().buffer_size() as usize,
            connections: vec![],
            frame_size: 0,
            client_name: self.config.client.name.clone(),
            output_name: self.config.server.system_name.clone(),
        };
        let _ = self
            .cbnet
            .status_tx
            .try_send(StatusMessageKind::JACKStatus(Some(jack_status.clone())));
    }
}
