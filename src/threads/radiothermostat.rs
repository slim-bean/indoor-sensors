
use std::sync::mpsc::Sender;
use Payload;

use std::thread;
use std::time::Duration;
use std::time::SystemTime;

use mio_httpc::CallBuilder;
use mio_httpc::Error as MioError;

use sensor_lib::ThermostatValue;
use std::fmt::{Display,Formatter};

#[derive(Debug)]
pub struct Error {
    message: String,
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "{}", self.message)
    }
}

pub struct RadioThermostat {
    sender: Sender<Payload>,
}

impl RadioThermostat {
    pub fn new(sender: Sender<Payload>) -> Result<RadioThermostat, Error> {
        Ok(RadioThermostat{
            sender,
        })
    }

    pub fn start_thread(mut thermostat: RadioThermostat) {
        thread::spawn(move || {
            info!("Started Thermostat Thread");
            loop {
                thread::sleep(Duration::from_secs(60));
                match query_thermostat() {
                    Ok(response) => {
                        match serde_json::from_slice::<ThermostatValue>(response.as_slice()){
                            Ok(mut val) => {
                                info!("Thermostat Val: {:?}", val);
                                val.timestamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64;
                                match serde_json::to_string(&val) {
                                    Ok(serial_val) => {
                                        match thermostat.sender.send(Payload{
                                            queue: String::from("/ws/2/grp/thermostat"),
                                            bytes: serial_val
                                        }){
                                            Ok(_) => {},
                                            Err(err) => {
                                                error!("Failed to send message to main thread: {}", err);
                                            },
                                        }
                                    }
                                    Err(err) => {
                                        error!("Failed to serialize the thermostat value: {}", err);
                                    }
                                };
                            },
                            Err(err) => {
                                error!("Failed to deserialize thermostat value: {}", err);
                            },
                        }
                    },
                    Err(err) => {
                        error!("Error querying thermostat: {}", err);
                    },
                }
            }


        });
    }
}

fn query_thermostat() -> Result<Vec<u8>, MioError> {
    let (response_meta, body) = CallBuilder::get().timeout_ms(20000).url("http://172.20.30.30/tstat")?.exec()?;
    Ok(body)
}