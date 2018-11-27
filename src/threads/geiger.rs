use std::fmt::{Display, Formatter};

use std::sync::mpsc::Sender;
use std::thread;
use std::time::SystemTime;
use std::time::Duration;
use std::path::Path;
use std::io::prelude::*;
use std::io::ErrorKind;
use std::collections::VecDeque;

use Payload;
use sensor_lib::SensorValue;

use serial::prelude::*;
use serial::unix::TTYPort;
use serial::core::Error as SerialError;


#[derive(Debug)]
pub struct Error {
    message: String,
}

impl From<SerialError> for Error {
    fn from(err: SerialError) -> Self {
        Error {
            message: format!("{}", err),
        }
    }
}


impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "{}", self.message)
    }
}

pub struct Geiger {
    sender: Sender<Payload>,
    port: TTYPort,
}

impl Geiger {
    pub fn new(sender: Sender<Payload>) -> Result<Geiger, Error> {
        info!("Setup radiation monitor serial port");
        let mut rad_port = serial::unix::TTYPort::open(Path::new("/dev/ttyUSB0"))?;
        let settings = serial::PortSettings {
            baud_rate: serial::Baud9600,
            char_size: serial::Bits8,
            parity: serial::ParityNone,
            stop_bits: serial::Stop1,
            flow_control: serial::FlowNone,
        };
        rad_port.configure(&settings)?;
        //We keep a very short timeout, if there isn't already data we don't want to wait for it
        rad_port.set_timeout(Duration::from_millis(10))?;
        Ok(Geiger {
            sender,
            port: rad_port,
        })
    }

    pub fn start_thread(mut geiger: Geiger) {
        thread::spawn(move || {
            info!("Started Geiger Thread");
            let mut cpm_queue = VecDeque::<u32>::with_capacity(60);
            let mut counter = 1;
            loop {

                //////////////////////////
                // RADIATION
                //////////////////////////

                //Sensor outputs every second, so if we do our read every second we should only have one value in the buffer
                thread::sleep(Duration::from_millis(1000));

                let mut buf: Vec<u8> = (0..200).collect();
                match geiger.port.read(&mut buf[..]) {
                    Ok(t) => {
                        let data = String::from_utf8_lossy(&buf[..t]);
                        let split_data: Vec<&str> = data.split(',').collect();

                        //Because we get out of sync pretty easily with the sensor barking data at us and buffers holding it, fast forward through the buffer until we find CPS
                        for i in 0..split_data.len() {
                            match split_data.get(i) {
                                Some(val) => {
                                    if val.trim() == "CPS" {
                                        match split_data.get(i + 3) {
                                            Some(cpm_string) => {
                                                match cpm_string.trim().parse::<u32>() {
                                                    Ok(cpm_incoming) => {
                                                        //We only want at most 60 elements, truncate anything older than 59 to make room for the new element
                                                        if cpm_queue.len() >= 60 {
                                                            cpm_queue.truncate(59);
                                                        }
                                                        cpm_queue.push_front(cpm_incoming);
                                                    }
                                                    Err(err) => {
                                                        error!("Failed to parse CPM value as integer: {}", err);
                                                    }
                                                }
                                            }
                                            None => {
                                                error!("Failed to get CPM data, maybe the packet was too short? {:?}", split_data);
                                            }
                                        }
                                        break;
                                    }
                                }
                                None => {}
                            }
                        }

//                        debug!("Received from radiation serial port: '{}', split: '{:?}'", data, split_data);
                    }
                    Err(e) => {
                        if e.kind() == ErrorKind::TimedOut {
                            //Nothing to do here, no data
                        } else {
                            warn!("Failed to read from serial port: {}", e);
                        }
                    }
                }

                //Send the value every minute
                if counter >= 60 {
                    let mut sum = 0;

                    debug!("Rad Sensor Queue: {:?}", cpm_queue);

                    for val in &cpm_queue {
                        sum = sum + val;
                    }

                    let temp_val = SensorValue {
                        id: 55,
                        timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                        value: (sum / cpm_queue.len() as u32).to_string(),
                    };

                    match serde_json::to_string(&temp_val) {
                        Ok(val) => {
                            match geiger.sender.send(Payload {
                                queue: String::from("/ws/2/grp/generic"),
                                bytes: val,
                            }) {
                                Ok(_) => {}
                                Err(err) => {
                                    error!("Failed to send message to main thread: {}", err);
                                }
                            }
                        }
                        Err(err) => {
                            error!("Failed to serialize the sensor value: {}", err);
                        }
                    };

                    counter = 0;
                }
                counter = counter + 1;
            }
        });
    }
}
