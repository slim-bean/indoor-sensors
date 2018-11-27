
use std::fmt::{Display,Formatter};

use std::sync::mpsc::Sender;
use std::sync::{Arc,Mutex};
use std::thread;
use std::time::SystemTime;
use std::time::Duration;

use Payload;
use sensor_lib::SensorValue;

use bme280::BME280;
use bme280::Error as Bme280Error;
use linux_hal::{I2cdev, Delay};
use linux_hal::i2cdev::linux::LinuxI2CError;



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

impl From<Bme280Error<LinuxI2CError>> for Error {
    fn from(err: Bme280Error<LinuxI2CError>) -> Self {
        match err {
            Bme280Error::CompensationFailed => {
                Error {
                    message: String::from("Compensation Failed"),
                }
            },
            Bme280Error::I2c(i2c_error) => {
                Error {
                    message: format!("BME Error: {}", i2c_error),
                }
            },
            Bme280Error::InvalidData => {
                Error {
                    message: String::from("Invalid Data"),
                }
            },
            Bme280Error::NoCalibrationData => {
                Error {
                    message: String::from("No Calibration Data"),
                }
            },
            Bme280Error::UnsupportedChip => {
                Error {
                    message: String::from("Unsupported Chip"),
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

pub struct Bmp280 {
    sender: Sender<Payload>,
    lock: Arc<Mutex<i32>>,
    bme280: BME280<I2cdev, Delay>,
}

impl Bmp280 {
    pub fn new(sender: Sender<Payload>, lock: Arc<Mutex<i32>>) -> Result<Bmp280, Error> {
        info!("Create and Init BMP280");
        let dev = I2cdev::new("/dev/i2c-1")?;
        //This "secondary" address is really the primary there is a little bit of a screw-up somewhere with this lib
        let mut bme280 = BME280::new_secondary(dev, Delay);
        bme280.init()?;

        Ok(Bmp280{
            sender,
            lock,
            bme280: bme280,
        })
    }

    pub fn start_thread(mut bmp280: Bmp280){
        thread::spawn( move|| {
            info!("Started BMP280 Thread");
            loop {


                //////////////////////////
                // AIR PRESSURE
                //////////////////////////

                //Read and send value every 5 mins
                thread::sleep(Duration::from_millis(299000));

                let mut measurements = None;

                match bmp280.lock.lock(){
                    Ok(_) => {
                        match bmp280.bme280.measure(){
                            Ok(result) => {
                                measurements = Some(result);
                            },
                            Err(err) => {
                                error!("Failed to read bmp280: {:?}", err);
                            },
                        }
                    },
                    Err(_) => {
                        error!("The lock has been poisoned, sending a poison message to kill the app");
                        bmp280.sender.send(Payload{
                            queue: String::from("poison"),
                            bytes: String::from("poison")
                        }).unwrap(); //We don't really care anymore if this thread panics
                    },
                }

                match measurements {
                    Some(measurement) => {
                        let mut buf = [b'\0'; 30];
                        //Convert to inches of mercury before sending
                        let len = dtoa::write(&mut buf[..], measurement.pressure/3386.389).unwrap();
                        let flt_as_string = std::str::from_utf8(&buf[..len]).unwrap();

                        let temp_val = SensorValue {
                            id: 54,
                            timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                            value: String::from(flt_as_string),
                        };

                        match serde_json::to_string(&temp_val) {
                            Ok(val) => {
                                match bmp280.sender.send(Payload{
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
            }
        });
    }
}