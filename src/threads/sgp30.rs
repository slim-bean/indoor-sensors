use std::fmt::{Display,Formatter};

use std::sync::mpsc::Sender;
use std::sync::{Arc,Mutex};
use std::thread;
use std::time::SystemTime;
use std::time::Duration;

use Payload;
use sensor_lib::SensorValue;

use linux_hal::{I2cdev, Delay};
use linux_hal::i2cdev::linux::LinuxI2CError;

use sgp30::{Sgp30 as Sgp, Humidity, Error as SgpError};

#[derive(Debug)]
pub struct Error {
    message: String,
}

impl From<LinuxI2CError> for Error {
    fn from(err: LinuxI2CError) -> Self {
        Error {
            message: format!("I2C Error: {}", err),
        }
    }
}

impl From<SgpError<LinuxI2CError>> for Error {
    fn from(err: SgpError<LinuxI2CError>) -> Self {
        match err {
            SgpError::I2c(i2c_error) => {
                Error {
                    message: format!("SGP30 I2C Error: {}", i2c_error),
                }
            },
            SgpError::Crc => {
                Error {
                    message: String::from("CRC Error"),
                }
            },
            SgpError::NotInitialized => {
                Error {
                    message: String::from("Not Initialized"),
                }
            },
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "{}", self.message)
    }
}

pub struct Sgp30 {
    sender: Sender<Payload>,
    lock: Arc<Mutex<i32>>,
    sgp30: Sgp<I2cdev, Delay>,
}

impl Sgp30 {
    pub fn new(sender: Sender<Payload>, lock: Arc<Mutex<i32>>) -> Result<Sgp30, Error> {
        info!("Create and Init SGP30");
        let dev2 = I2cdev::new("/dev/i2c-1")?;
        let address = 0x58;
        let mut sgp30 = Sgp::new(dev2, address, Delay);
        sgp30.init()?;
        Ok(Sgp30{
            sender,
            lock,
            sgp30
        })
    }

    pub fn start_thread(mut sgp: Sgp30){
        thread::spawn(move || {
            info!("Started SGP30 Thread");
            let mut counter = 1;
            loop {

                //////////////////////////
                // CO2 and VOC
                //////////////////////////


                // The driver for the SGP30 says we have to call the measure() function once a second
                // for the dynamic baseline to work properly, the main loop runs on a one sec delay
                // but we don't really need one sec resolution on this sensor, so we call the measure function
                // but usually ignore the result.
                // A measurement is supposed to take 12 ms so we will subtract that from our sleep time
                thread::sleep(Duration::from_millis(1000 - 12));

                let mut measurement = None;
                match sgp.lock.lock() {
                    Ok(_) => {
                        match sgp.sgp30.measure() {
                            Ok(val) => {
                                measurement = Some(val);
                            },
                            Err(err) => {
                                error!("Failed to read from SGP30: {:?}", err);
                            },
                        }
                    },
                    Err(_) => {
                        error!("The lock has been poisoned, sending a poison message to kill the app");
                        sgp.sender.send(Payload{
                            queue: String::from("poison"),
                            bytes: String::from("poison")
                        }).unwrap(); //We don't really care anymore if this thread panics
                    },
                }

                //Even though we read the value once a second, only send it once a minute
                if counter >= 60 {
                    match measurement {
                        Some(val) => {

                            let temp_val = SensorValue {
                                id: 52,
                                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                                value: val.co2eq_ppm.to_string(),
                            };


                            match serde_json::to_string(&temp_val) {
                                Ok(val) => {
                                    match sgp.sender.send(Payload{
                                        queue: String::from("/ws/2/grp/generic"),
                                        bytes: val
                                    }){
                                        Ok(_) => {},
                                        Err(err) => {
                                            error!("Failed to send message to main thread: {}", err);
                                        },
                                    }
                                }
                                Err(err) => {
                                    error!("Failed to serialize the sensor value: {}", err);
                                }
                            };


                            let temp_val = SensorValue {
                                id: 53,
                                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                                value: val.tvoc_ppb.to_string(),
                            };


                            match serde_json::to_string(&temp_val) {
                                Ok(val) => {
                                    match sgp.sender.send(Payload{
                                        queue: String::from("/ws/2/grp/generic"),
                                        bytes: val
                                    }){
                                        Ok(_) => {},
                                        Err(err) => {
                                            error!("Failed to send message to main thread: {}", err);
                                        },
                                    }
                                }
                                Err(err) => {
                                    error!("Failed to serialize the sensor value: {}", err);
                                }
                            };

                        },
                        None => {},
                    }
                    counter = 0;
                }
                counter = counter + 1;
            }
        });
    }
}