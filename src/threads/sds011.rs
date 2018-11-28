use std::fmt::{Display, Formatter};

use std::sync::mpsc::Sender;
use std::thread;
use std::time::SystemTime;
use std::time::Duration;
use std::path::Path;
use std::io::prelude::*;
use std::io::Error as IoError;
use std::collections::VecDeque;

use Payload;
use sensor_lib::AirParticulateValue;

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

impl From<IoError> for Error {
    fn from(err: IoError) -> Self {
        Error{
            message: format!("{}", err),
        }
    }
}


impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "{}", self.message)
    }
}

pub struct Sds011 {
    sender: Sender<Payload>,
    port: TTYPort,
}

impl Sds011 {
    pub fn new(sender: Sender<Payload>) -> Result<Sds011, Error> {
        info!("Setup air quality monitor serial port");
        let mut air_port = serial::unix::TTYPort::open(Path::new("/dev/serial0"))?;
        let settings = serial::PortSettings {
            baud_rate: serial::Baud9600,
            char_size: serial::Bits8,
            parity: serial::ParityNone,
            stop_bits: serial::Stop1,
            flow_control: serial::FlowNone,
        };
        air_port.configure(&settings)?;
        air_port.set_timeout(Duration::from_millis(250))?;

        //for some reason we get a bogus response the first time we send a command after startup, so we retry this guy a few times
        for _x in 0..3 {
            info!("Changing air monitor to query only mode");
            let cmd = [0xAAu8, 0xB4, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x02, 0xAB];
            air_port.write(&cmd)?;

            //These commands seem to take some time to process, so we just wait a little bit before checking for a response
            thread::sleep(Duration::from_millis(500));

            //Read the response
            let mut buf: Vec<u8> = (0..20).collect();
            match air_port.read(&mut buf[..]) {
                Ok(t) => {
                    if t == 10 {
                        if buf.get(1) == Some(&0xC5) && buf.get(2) == Some(&0x02) && buf.get(3) == Some(&0x01) && buf.get(4) == Some(&0x01) {
                            info!("Air Particulate Sensor Successfully Changed to query only mode");
                            break;
                        } else {
                            error!("Failed to put air sensor into query only mode, Received: {:?}", buf);
                        }
                    } else {
                        error!("Failed to put air sensor into query only mode: Packet from air sensor was incorrect length: {:?}", buf);
                    }
                }
                Err(e) => {
                    error!("Failed to read from air sensor serial port: {}", e);
                }
            }
        }


        info!("Turning off air monitor");
        let cmd = [0xAAu8, 0xB4, 0x06, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x05, 0xAB];
        air_port.write(&cmd)?;

        //These commands seem to take some time to process, so we just wait a little bit before checking for a response
        thread::sleep(Duration::from_millis(500));

        let mut buf: Vec<u8> = (0..20).collect();
        match air_port.read(&mut buf[..]) {
            Ok(t) => {
                if t == 10 {
                    if buf.get(1) == Some(&0xC5) && buf.get(2) == Some(&0x06) && buf.get(3) == Some(&0x01) && buf.get(4) == Some(&0x00) {
                        info!("Air Particulate Sensor Successfully Turned Off")
                    } else {
                        error!("Failed to turn off air sensor!, Received: {:?}", buf);
                    }
                } else {
                    error!("Failed to turn off air sensor: Packet from air sensor was incorrect length: {:?}", buf);
                }
            }
            Err(e) => {
                error!("Failed to read from air sensor serial port: {}", e);
            }
        }

        Ok(Sds011 {
            sender,
            port: air_port,
        })
    }

    pub fn start_thread(mut sds011: Sds011) {
        info!("Started SDS011 Thread");
        let mut counter = 0;
        let mut pm2_5_queue = VecDeque::<i32>::with_capacity(30);
        let mut pm10_queue = VecDeque::<i32>::with_capacity(30);
        thread::spawn(move || {
            loop {

                // The datasheet says the sensor has a lifespan of 8000 hours, which if we left it on all the time would not last us very long, about a year.
                // So to extend the life we are going to basically go with a dutycycle of being on for one minute out of every five minutes
                // This should result in 24hrs * 60mins = 1440 mins/day
                // 1440 / 5 = 288 five min intervals, where we are on for one minute.
                // Which should add up to being on for 288 mins a day
                // 8000 hrs = 480000 mins, 480000/288 = 1666 days or about 4.5 years


                thread::sleep(Duration::from_millis(1000));

                //One minute before the five minute mark turn on the air sensor, this gives 30 seconds to let the sensor stabilize as recommended by datasheet
                if counter == 240 {
                    //Send the command to turn on the sensor
                    let cmd = [0xAAu8, 0xB4, 0x06, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x06, 0xAB];
                    match sds011.port.write(&cmd){
                        Ok(_) => {},
                        Err(err) => {
                            //TODO what do we do if this happens???
                            error!("Failed to write to air sensor: {}", err)
                        },
                    }
                    //These commands seem to take some time to process, so we just wait a little bit before checking for a response
                    thread::sleep(Duration::from_millis(500));

                    let mut buf: Vec<u8> = (0..20).collect();
                    match sds011.port.read(&mut buf[..]) {
                        Ok(t) => {
                            if t == 10 {
                                if buf.get(1) == Some(&0xC5) && buf.get(2) == Some(&0x06) && buf.get(3) == Some(&0x01) && buf.get(4) == Some(&0x01) {
                                    info!("Air Particulate Sensor Successfully Turned On")
                                } else {
                                    error!("Failed to turn on air sensor!, Received: {:?}", buf);
                                }
                            } else {
                                error!("Failed to turn on air sensor: Packet from air sensor was incorrect length: {:?}", buf);
                            }
                        }
                        Err(e) => {
                            error!("Failed to read from air sensor serial port: {}", e);
                        }
                    }
                }

                //Accumulate 30 seconds of readings
                if counter >= 270 && counter < 300 {
                    //Send the command to query data
                    let cmd = [0xAAu8, 0xB4, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x02, 0xAB];
                    match sds011.port.write(&cmd){
                        Ok(_) => {},
                        Err(err) => {
                            //TODO what do we do if this happens???
                            error!("Failed to write to air sensor: {}", err)
                        },
                    }
                    //Wait just a little bit before checking result
                    thread::sleep(Duration::from_millis(500));
                    //Read the response
                    let mut buf: Vec<u8> = (0..20).collect();
                    match sds011.port.read(&mut buf[..]) {
                        Ok(t) => {
                            if t == 10 && buf.get(0) == Some(&0xAA) && buf.get(1) == Some(&0xC0){
                                if pm2_5_queue.len() >= 30 {
                                    pm2_5_queue.truncate(29);
                                }
                                if pm10_queue.len() >= 30 {
                                    pm10_queue.truncate(29);
                                }
                                pm2_5_queue.push_front(((buf[3] as i32) << 8) + ((buf[2] as i32) << 0));
                                pm10_queue.push_front(((buf[5] as i32) << 8) + ((buf[4] as i32) << 0));
                            } else {
                                warn!("Packet from air sensor was incorrect length or did not start with the correct bytes: {:?}", buf);
                            }
//                            debug!("Received from air sensor: '{:?}'", buf);
                        }
                        Err(e) => {
                            warn!("Failed to read from air sensor serial port: {}", e);
                        }
                    }
                }

                //Every five minutes take the samples from the air particulate sensor and send them
                if counter >= 300 {
                    debug!("2.5 Queue: {:?}", pm2_5_queue);
                    let mut sum: i32 = 0;
                    for val in &pm2_5_queue {
                        sum = sum + val;
                    }
                    let pm2_5 = sum / pm2_5_queue.len() as i32;

                    debug!("10 Queue: {:?}", pm10_queue);
                    let mut sum: i32 = 0;
                    for val in &pm10_queue {
                        sum = sum + val;
                    }
                    let pm10 = sum / pm10_queue.len() as i32;

                    let air_part_val = AirParticulateValue{
                        timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                        location: 2,
                        pm2_5: pm2_5 as i16,
                        pm10: pm10 as i16,
                    };

                    match serde_json::to_string(&air_part_val) {
                        Ok(val) => {
                            match sds011.sender.send(Payload{
                                queue: String::from("/ws/2/grp/air_particulate"),
                                bytes: val
                            }){
                                Ok(_) => {},
                                Err(err) => {
                                    error!("Failed to send message to main thread: {}", err);
                                },
                            }
                        }
                        Err(err) => {
                            error!("Failed to serialize the AirParticulateValue: {}", err);
                        }
                    };

                    info!("Turning off air monitor");
                    let cmd = [0xAAu8, 0xB4, 0x06, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x05, 0xAB];
                    match sds011.port.write(&cmd) {
                        Ok(_) => {},
                        Err(err) => {
                            //TODO what do we do if this happens???
                            error!("Failed to write to air sensor: {}", err)
                        },
                    }

                    //These commands seem to take some time to process, so we just wait a little bit before checking for a response
                    thread::sleep(Duration::from_millis(500));

                    let mut buf: Vec<u8> = (0..20).collect();
                    match sds011.port.read(&mut buf[..]) {
                        Ok(t) => {
                            if t == 10 {
                                if buf.get(1) == Some(&0xC5) && buf.get(2) == Some(&0x06) && buf.get(3) == Some(&0x01) && buf.get(4) == Some(&0x00) {
                                    info!("Air Particulate Sensor Successfully Turned Off")
                                } else {
                                    error!("Failed to turn off air sensor!, Received: {:?}", buf);
                                }
                            } else {
                                error!("Failed to turn off air sensor: Packet from air sensor was incorrect length: {:?}", buf);
                            }
                        }
                        Err(e) => {
                            error!("Failed to read from air sensor serial port: {}", e);
                        }
                    }


                    //Reset the counter after the five minute mark
                    counter = 0;
                }

                counter = counter + 1;
            }

        });
    }
}