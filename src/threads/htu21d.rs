use std::fmt::{Display,Formatter};

use std::sync::mpsc::Sender;
use std::sync::{Arc,Mutex};
use std::thread;
use std::time::SystemTime;
use std::time::Duration;

use Payload;
use sensor_lib::TempHumidityValue;

use linux_hal::{I2cdev, Delay};
use linux_hal::i2cdev::linux::LinuxI2CError;
use htu21d::HTU21D;
use htu21d::Error as Htu21dError;

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

impl From<Htu21dError<LinuxI2CError>> for Error {
    fn from(err: Htu21dError<LinuxI2CError>) -> Self {
        match err {
            Htu21dError::I2c(i2c_error) => {
                Error {
                    message: format!("HTU21 I2C Error: {}", i2c_error),
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


pub struct Htu21d {
    sender: Sender<Payload>,
    lock: Arc<Mutex<i32>>,
    htu21d: HTU21D<I2cdev, Delay>,
}

impl Htu21d {
    pub fn new(sender: Sender<Payload>, lock: Arc<Mutex<i32>>) -> Result<Htu21d, Error> {
        info!("Create and Init HTU21Df");
        let dev = I2cdev::new("/dev/i2c-1")?;
        let mut htu21d = HTU21D::new(dev, Delay);
        htu21d.reset()?;

        Ok(Htu21d{
            sender,
            lock,
            htu21d
        })
    }

    pub fn start_thread(mut htu: Htu21d){
        thread::spawn(move || {
            info!("Started HTU21D Thread");
            loop {
                //Read and send value every 1 mins (add a few milliseconds to hopefully reduce collisions on mutex blocking)
                thread::sleep(Duration::from_millis(60005));

                //////////////////////////
                // TEMP AND HUMIDITY
                //////////////////////////

                let mut temp = None;
                let mut humidity = None;

                match htu.lock.lock() {
                    Ok(_) => {
                        match htu.htu21d.read_temperature() {
                            Ok(val) => {
                                temp = Some(val);
                            },
                            Err(err) => {
                                error!("Failed to read temp from HTU21D: {:?}", err);
                            },
                        }
                        match htu.htu21d.read_humidity() {
                            Ok(val) => {
                                humidity = Some(val);
                            },
                            Err(err) => {
                                error!("Failed to read humidity from HTU21D: {:?}", err);
                            },
                        }
                    },
                    Err(_) => {
                        error!("The lock has been poisoned, sending a poison message to kill the app");
                        htu.sender.send(Payload{
                            queue: String::from("poison"),
                            bytes: String::from("poison")
                        }).unwrap(); //We don't really care anymore if this thread panics
                    },
                }

                if temp.is_some() && humidity.is_some() {
                    let temp_f = temp.unwrap() as f32 * 1.8 + 32.0;

                    //Set the humidity value in the SGP30
                    //This equation for absolute humidity comes from: https://carnotcycle.wordpress.com/2012/08/04/how-to-convert-relative-humidity-to-absolute-humidity/
//                let abs_humidity = (6.112 * E.powf(((17.67 * temp)/(temp+243.5)) as f64) as f32 * humidity * 2.1674) as f32 /(273.15+temp);
//                let sgp30_humidity = Humidity::from_f32(abs_humidity).unwrap();
//                sgp.set_humidity(Some(&sgp30_humidity)).unwrap();


                    let temp_humidity = TempHumidityValue {
                        timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                        location: 2,
                        temp: temp_f,
                        humidity: humidity.unwrap(),
                    };

                    match serde_json::to_string(&temp_humidity) {
                        Ok(val) => {
                            match htu.sender.send(Payload{
                                queue: String::from("/ws/2/grp/temp_humidity"),
                                bytes: val
                            }){
                                Ok(_) => {},
                                Err(err) => {
                                    error!("Failed to send message to main thread: {}", err);
                                },
                            }
                        }
                        Err(err) => {
                            error!("Failed to serialize the temp_humidity value: {}", err);
                        }
                    };
                }



            }
        });
    }

}