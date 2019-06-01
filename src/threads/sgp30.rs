use std::fmt::{Display, Formatter};

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;
use std::time::Duration;
use std::f64::consts::E;
use std::f32::NAN;
use std::fs;
use std::path::Path;
use std::io::Error as IoError;
use std::num::ParseIntError;
use std::collections::VecDeque;


use Payload;
use sensor_lib::SensorValue;

use linux_hal::{I2cdev, Delay};
use linux_hal::i2cdev::linux::LinuxI2CError;

use sgp30::{Sgp30 as Sgp, Humidity, Error as SgpError, Baseline};

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
            }
            SgpError::Crc => {
                Error {
                    message: String::from("CRC Error"),
                }
            }
            SgpError::NotInitialized => {
                Error {
                    message: String::from("Not Initialized"),
                }
            }
        }
    }
}

impl From<IoError> for Error {
    fn from(err: IoError) -> Self {
        Error {
            message: format!("{}", err),
        }
    }
}

impl From<ParseIntError> for Error {
    fn from(err: ParseIntError) -> Self {
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

pub struct Sgp30 {
    sender: Sender<Payload>,
    lock: Arc<Mutex<i32>>,
    sgp30: Sgp<I2cdev, Delay>,
    humidity_mutex: Arc<Mutex<(f32, f32)>>,
}

impl Sgp30 {
    pub fn new(sender: Sender<Payload>, lock: Arc<Mutex<i32>>, humidity_mutex: Arc<Mutex<(f32, f32)>>) -> Result<Sgp30, Error> {
        info!("Create and Init SGP30");
        let dev2 = I2cdev::new("/dev/i2c-1")?;
        let address = 0x58;
        let mut sgp30 = Sgp::new(dev2, address, Delay);
        sgp30.init()?;

        let co2_path = Path::new("/var/lib/indoor_sensors/sgp30_co2.txt");
        let tvoc_path = Path::new("/var/lib/indoor_sensors/sgp30_tvoc.txt");

        if co2_path.exists() && tvoc_path.exists() {
            match read_baseline(&co2_path, &tvoc_path) {
                Ok(baseline) => {
                    sgp30.set_baseline(&baseline)?;
                }
                Err(err) => {
                    error!("Failed to read existing baseline values for SGP30: {}, no baseline will be used", err)
                }
            }
        } else {
            info!("No existing baseline files found for SGP30, no baseline will be used")
        }

        Ok(Sgp30 {
            sender,
            lock,
            sgp30,
            humidity_mutex,
        })
    }

    pub fn start_thread(mut sgp: Sgp30) {
        thread::spawn(move || {
            info!("Started SGP30 Thread");
            let mut counter = 1;
            let mut co2_queue = VecDeque::<u16>::with_capacity(60);
            let mut voc_queue = VecDeque::<u16>::with_capacity(60);
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
                            }
                            Err(err) => {
                                error!("Failed to read from SGP30: {:?}", err);
                            }
                        }
                    }
                    Err(_) => {
                        error!("The lock has been poisoned, sending a poison message to kill the app");
                        sgp.sender.send(Payload {
                            queue: String::from("poison"),
                            bytes: String::from("poison"),
                        }).unwrap(); //We don't really care anymore if this thread panics
                    }
                }

                if let Some(meas) = measurement {
                    if co2_queue.len() >= 60 {
                        co2_queue.truncate(59);
                    }
                    if voc_queue.len() >= 60 {
                        voc_queue.truncate(59);
                    }
                    co2_queue.push_front(meas.co2eq_ppm);
                    voc_queue.push_front(meas.tvoc_ppb);
                }

                //Even though we read the value once a second, only send it once a minute
                if counter >= 60 {
                    debug!("CO2 Vals: {:?}", co2_queue);
                    debug!("VOC Vals: {:?}", voc_queue);

                    let mut sum = 0u32;
                    for val in &co2_queue {
                        sum = sum + *val as u32;
                    }
                    let co2_avg = sum / co2_queue.len() as u32;

                    let mut sum = 0u32;
                    for val in &voc_queue {
                        sum = sum + *val as u32;
                    }

                    let voc_avg = sum / voc_queue.len() as u32;

                    let temp_val = SensorValue {
                        id: 52,
                        timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                        value: co2_avg.to_string(),
                    };

                    match serde_json::to_string(&temp_val) {
                        Ok(val) => {
                            match sgp.sender.send(Payload {
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


                    let temp_val = SensorValue {
                        id: 53,
                        timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                        value: voc_avg.to_string(),
                    };


                    match serde_json::to_string(&temp_val) {
                        Ok(val) => {
                            match sgp.sender.send(Payload {
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


                    //Update the humidity value for the next set of readings:
                    //This equation for absolute humidity comes from: https://carnotcycle.wordpress.com/2012/08/04/how-to-convert-relative-humidity-to-absolute-humidity/
                    let mut temp = NAN;
                    let mut humidity = NAN;
                    match sgp.humidity_mutex.lock() {
                        Ok(mut_val) => {
                            temp = mut_val.0;
                            humidity = mut_val.1;
                        }
                        Err(_) => {}
                    }

                    if !temp.is_nan() && !humidity.is_nan() {
                        let abs_humidity = (6.112 * E.powf(((17.67 * temp) / (temp + 243.5)) as f64) as f32 * humidity * 2.1674) as f32 / (273.15 + temp);
                        match Humidity::from_f32(abs_humidity) {
                            Ok(sgp30_humidity) => {
                                match sgp.lock.lock() {
                                    Ok(_) => {
                                        match sgp.sgp30.set_humidity(Some(&sgp30_humidity)) {
                                            Ok(_) => {
                                                debug!("Set SGP abs humidity to: {} from a temp val of {} and humidity val of {}", abs_humidity, temp, humidity);
                                            }
                                            Err(err) => {
                                                error!("Failed to update the humidity value of the sgp30: {:?}", err);
                                            }
                                        }
                                    }
                                    Err(_) => {
                                        //I'm ignoring this lock failure because we will catch it above if the lock is poisoned
                                    }
                                }
                            }
                            Err(err) => {
                                error!("Failed to create a humidity value for SGP30: {:?}", err);
                            }
                        }
                    }

                    //Save off the baseline data:
                    let mut baseline = None;
                    match sgp.lock.lock() {
                        Ok(_) => {
                            match sgp.sgp30.get_baseline() {
                                Ok(incoming_baseline) => {
                                    baseline = Some(incoming_baseline);
                                }
                                Err(err) => {
                                    error!("Failed to read the baseline data from the sgp30: {:?}", err);
                                }
                            }
                        }
                        Err(_) => {
                            //I'm ignoring this lock failure because we will catch it above if the lock is poisoned
                        }
                    }

                    if let Some(base) = baseline {
                        match fs::write("/var/lib/indoor_sensors/sgp30_co2.txt", base.co2eq.to_string()) {
                            Ok(_) => {}
                            Err(err) => {
                                error!("Failed to save sgp30 baseline to a file: {}", err);
                            }
                        }
                        match fs::write("/var/lib/indoor_sensors/sgp30_tvoc.txt", base.tvoc.to_string()) {
                            Ok(_) => {}
                            Err(err) => {
                                error!("Failed to save sgp30 baseline to a file: {}", err);
                            }
                        }
                        debug!("Saved CO2 baseline of {} and TVOC baseline of {}", base.co2eq, base.tvoc);
                    }

                    counter = 0;
                }
                counter = counter + 1;
            }
        });
    }
}

fn read_baseline(co2_path: &Path, tvoc_path: &Path) -> Result<Baseline, Error> {
    let co2_val = fs::read_to_string(co2_path)?;
    let tvoc_val = fs::read_to_string(tvoc_path)?;
    debug!("Read raw string values of CO2: '{}' and TVOC: '{}'", co2_val, tvoc_val);
    let co2_u16 = co2_val.trim().parse::<u16>()?;
    let tvoc_u16 = tvoc_val.trim().parse::<u16>()?;

    let baseline = Baseline {
        co2eq: co2_u16,
        tvoc: tvoc_u16,
    };
    info!("Found baseline values for SGP30, CO2: {}, TVOC: {}", co2_u16, tvoc_u16);
    Ok(baseline)
}