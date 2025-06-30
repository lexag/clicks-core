use std::sync::Weak;

use crate::{
    audio::{
        config::AudioConfig, notification::JACKNotificationHandler, processor::AudioProcessor,
        source::SourceConfig,
    },
    logger,
};
use common::network::{JACKStatus, StatusMessageKind};
use crossbeam_channel::{Receiver, Sender};
use jack::{
    AsyncClient, AudioIn, AudioOut, Client, ClientOptions, ClientStatus, Error, Port, PortFlags,
    Unowned,
};

use common::command::ControlCommand;

pub struct AudioHandler {
    pub client: AsyncClient<JACKNotificationHandler, AudioProcessor>,
    pub num_sources: usize,
    config: common::config::AudioConfiguration,
    jack_server_process: std::process::Child,
}

impl AudioHandler {
    pub fn new(
        config: common::config::AudioConfiguration,
        sources: Vec<SourceConfig>,
        rx: Receiver<ControlCommand>,
        tx_loopback: Sender<ControlCommand>,
        tx: Sender<StatusMessageKind>,
    ) -> AudioHandler {
        let jack_server_process = std::process::Command::new("jackd")
            .arg("-R")
            .args(["-d", "alsa"])
            .args(["-d", &config.server.device_name])
            .args(["-r", &config.server.sample_rate.to_string()])
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(5));

        let client_res = Client::new(
            &config.client.name.to_string(),
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

                let mut ports: (Vec<Port<AudioOut>>, Vec<Port<Unowned>>) = (vec![], vec![]);

                // Register io_matrix.0 amount of ports on the client and save for processor
                // reference
                for c_out_idx in 0..sources.len() {
                    ports.0.push(
                        client
                            .register_port(&c_out_idx.to_string(), AudioOut::default())
                            .expect("Port register failed"),
                    );
                }

                // Populate ports.1 with io_matrix.1 amount of ports on the system output. If
                // io_matrix.1 > number of physical ports, overflow and map port 0 again, then port
                // 1 etc.

                let mut o_ports = client.ports(
                    Some(&config.server.system_name),
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

                logger::log(
                    format!("Found {} system ports.", ports.1.len()),
                    logger::LogContext::AudioHandler,
                    logger::LogKind::Note,
                );
                let jack_status = JACKStatus {
                    io_size: (sources.len(), config.server.num_channels),
                    sample_rate: client.sample_rate(),
                    buffer_size: client.buffer_size() as usize,
                    connections: vec![],
                    frame_size: 0,
                    client_name: config.client.name.clone(),
                    output_name: config.server.system_name.clone(),
                };
                let num_sources = sources.len();
                let _ = tx.try_send(StatusMessageKind::JACKStatus(Some(jack_status.clone())));
                let processor = AudioProcessor::new(sources, ports, rx, tx_loopback, tx);
                let ac = client
                    .activate_async(JACKNotificationHandler, processor)
                    .unwrap();
                let ah = AudioHandler {
                    client: ac,
                    num_sources,
                    config,
                    jack_server_process,
                };

                return ah;
            }
        }
    }

    fn get_ports(&self) -> (Vec<Port<Unowned>>, Vec<Port<Unowned>>) {
        let mut ports: (Vec<Port<Unowned>>, Vec<Port<Unowned>>) = (vec![], vec![]);
        let mut i_ports = self.client.as_client().ports(
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
            .map(|name| self.client.as_client().port_by_name(name).unwrap())
            .collect();

        let mut o_ports = self.client.as_client().ports(
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
            .map(|name| self.client.as_client().port_by_name(name).unwrap())
            .collect();

        return ports;
    }

    pub fn try_route_ports(&self, from: usize, to: usize, connect: bool) -> bool {
        let ports = self.get_ports();
        let p_from = ports.0[from].clone();
        let p_to = ports.1[to].clone();
        let res = if connect {
            self.client.as_client().connect_ports(&p_from, &p_to)
        } else {
            self.client.as_client().disconnect_ports(&p_from, &p_to)
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

    pub fn get_jack_status(&self) -> JACKStatus {
        let ports = self.get_ports();
        return JACKStatus {
            io_size: (self.num_sources, self.config.server.num_channels),
            buffer_size: self.client.as_client().buffer_size() as usize,
            sample_rate: self.client.as_client().sample_rate(),
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
        };
    }

    pub fn shutdown(&mut self) {
        let _ = self.jack_server_process.kill();
    }
}
