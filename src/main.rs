#![feature(duration_as_u128)]

#[macro_use]
extern crate log;
extern crate fern;
extern crate chrono;

extern crate embedded_hal as hal;
extern crate linux_embedded_hal as linux_hal;

extern crate mosquitto_client;
#[macro_use]
extern crate serde_derive;
extern crate serde_yaml;
extern crate serde_json;
extern crate dtoa;

extern crate htu21d;
extern crate sgp30;
extern crate bme280;
extern crate sensor_lib;

extern crate serial;
use serial::prelude::*;

use linux_hal::{I2cdev, Delay};
use htu21d::HTU21D;
use sgp30::{Sgp30, Measurement, Humidity};
use bme280::BME280;

use std::thread;
use std::time::Duration;
use std::path::Path;
use std::io::prelude::*;

use std::time::SystemTime;

use std::f64::consts::E;

use sensor_lib::{SensorValue, TempHumidityValue, AirParticulateValue, load_from_file};

use mosquitto_client::Mosquitto;

fn main() {
    setup_logger().unwrap();
    info!("Starting indoor_sensors");

    info!("Loading sensors YAML file");
    let sensors = load_from_file("/etc/indoor_sensors/sensors.yml");

    info!("Opening I2C device");
    let dev = I2cdev::new("/dev/i2c-1").unwrap();

    info!("Create and reset HTU21D");
    let mut htu21d = HTU21D::new(dev, Delay);
    htu21d.reset().unwrap();

    info!("Create and reset SGP30");
    let dev2 = I2cdev::new("/dev/i2c-1").unwrap();
    let address = 0x58;
    let mut sgp = Sgp30::new(dev2, address, Delay);
    sgp.init().unwrap();

    info!("Create and init BMP280");
    // using Linux I2C Bus #1 in this example
    let dev3 = I2cdev::new("/dev/i2c-1").unwrap();
    //This "secondary" address is really the primary there is a little bit of a screw-up somewhere with this lib
    let mut bme280 = BME280::new_secondary(dev3, Delay);
    bme280.init().unwrap();

    info!("Setup radiation monitor serial port");
    let mut rad_port = serial::unix::TTYPort::open(Path::new("/dev/ttyUSB0")).unwrap();
    let settings = serial::PortSettings {
        baud_rate:    serial::Baud9600,
        char_size:    serial::Bits8,
        parity:       serial::ParityNone,
        stop_bits:    serial::Stop1,
        flow_control: serial::FlowNone,
    };
    rad_port.configure(&settings).unwrap();
    rad_port.set_timeout(Duration::from_millis(100)).unwrap();

    info!("Setup air quality monitor serial port");
    let mut air_port = serial::unix::TTYPort::open(Path::new("/dev/serial0")).unwrap();
    let settings = serial::PortSettings {
        baud_rate:    serial::Baud9600,
        char_size:    serial::Bits8,
        parity:       serial::ParityNone,
        stop_bits:    serial::Stop1,
        flow_control: serial::FlowNone,
    };
    air_port.configure(&settings).unwrap();
    air_port.set_timeout(Duration::from_millis(250)).unwrap();

    info!("Changing air monitor to query only mode");
    let cmd = [0xAAu8, 0xB4, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x02, 0xAB];
    air_port.write(&cmd).unwrap();
    let mut buf: Vec<u8> = (0..50).collect();
    air_port.read(&mut buf[..]).unwrap();
    info!("Received {:?}", buf);



    info!("Connecting to MQ");
    let m = mosquitto_client::Mosquitto::new("indoor_sensors");
    m.connect("localhost", 1883).unwrap();
    let mut counter = 0;
    //TODO do we want to average this over the minute instead of taking the instantaneous reading?
    let mut cpm = 0u16;

    loop {


        // The driver for the SGP30 says we have to call the measure() function once a second
        // for the dynamic baseline to work properly, the main loop runs on a one sec delay
        // but we don't really need one sec resolution on this sensor, so we call the measure function
        // but usually ignore the result.

        let measurement: Measurement = sgp.measure().unwrap();
        debug!("CO₂eq parts per million: {}", measurement.co2eq_ppm);
        debug!("TVOC parts per billion: {}", measurement.tvoc_ppb);


        //We also read the serial port for the radiation sensor every second because that's the rate it outputs and this will keep our value current
        let mut buf: Vec<u8> = (0..255).collect();
        match rad_port.read(&mut buf[..]) {
            Ok(t) => {
                let data = String::from_utf8_lossy(&buf[..t]);
                let split_data: Vec<&str> = data.split(',').collect();
                if split_data.len() >= 4 {
                    match split_data.get(3) {
                        Some(cpm_string) => {
                            match cpm_string.trim().parse::<u16>(){
                                Ok(cpm_incoming) => {
                                    cpm = cpm_incoming;
                                },
                                Err(err) => {
                                    error!("Failed to parse CPM value as integer: {}", err);
                                }

                            }
                        },
                        None => {
                            error!("Failed to get the third index from the rad sensor data");
                        },
                    }

                }
                debug!("Received from radiation serial port: '{}', split: '{:?}'", data, split_data);
            }
            Err(e) => {
                warn!("Failed to read from serial port: {}", e);
            }
        }

        //Send the command to query data
        let cmd = [0xAAu8, 0xB4, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x02, 0xAB];
        air_port.write(&cmd).unwrap();
        //Read the response
        let mut buf: Vec<u8> = (0..20).collect();
        match air_port.read(&mut buf[..]) {
            Ok(t) => {
                if t == 10 {
                    let pm2_5 = ((buf[3] as i16) << 8) + ((buf[2] as i16) << 0);
                    let pm10 = ((buf[5] as i16) << 8) + ((buf[4] as i16) << 0);
                    info!("PM2.5: {}, PM10: {}", pm2_5, pm10);

                    let air_part_val = AirParticulateValue{
                        timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                        location: 2,
                        pm2_5,
                        pm10
                    };

                    match serde_json::to_string(&air_part_val) {
                        Ok(val) => {
                            send_to_topic(&m, "/ws/2/grp/air_particulate", val.as_bytes());
                        }
                        Err(err) => {
                            error!("Failed to serialize the temp_humidity value: {}", err);
                        }
                    };

                } else {
                    warn!("Packet from air sensor was incorrect length: {:?}", buf);
                }
                debug!("Received from air sensor: '{:?}'", buf);
            }
            Err(e) => {
                warn!("Failed to read from air sensor serial port: {}", e);
            }
        }


        //Send values once a minute
        if counter > 60 {


            //////////////////////////
            // TEMP AND HUMIDITY
            //////////////////////////

            let temp = htu21d.read_temperature().unwrap();
            let temp_f = temp as f32 * 1.8 + 32.0;
            let humidity = htu21d.read_humidity().unwrap();

            //Set the humidity value in the SGP30
            //This equation for absolute humidity comes from: https://carnotcycle.wordpress.com/2012/08/04/how-to-convert-relative-humidity-to-absolute-humidity/
            let abs_humidity = (6.112 * E.powf(((17.67 * temp)/(temp+243.5)) as f64) as f32 * humidity * 2.1674) as f32 /(273.15+temp);
            let sgp30_humidity = Humidity::from_f32(abs_humidity).unwrap();
            sgp.set_humidity(Some(&sgp30_humidity)).unwrap();

            let mut buf = [b'\0'; 30];
            let len = dtoa::write(&mut buf[..], temp_f).unwrap();
            let flt_as_string = std::str::from_utf8(&buf[..len]).unwrap();

            let temp_val = SensorValue {
                id: 50,
                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                value: String::from(flt_as_string),
            };

            for queue in &sensors.get(&50i16).unwrap().destination_queues {
                match serde_json::to_string(&temp_val) {
                    Ok(val) => {
                        send_to_topic(&m, &queue, val.as_bytes());
                    }
                    Err(err) => {
                        error!("Failed to serialize the sensor value: {}", err);
                    }
                };
            };

            let mut buf = [b'\0'; 30];
            let len = dtoa::write(&mut buf[..], humidity).unwrap();
            let flt_as_string = std::str::from_utf8(&buf[..len]).unwrap();

            let temp_val = SensorValue {
                id: 51,
                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                value: String::from(flt_as_string),
            };

            for queue in &sensors.get(&51i16).unwrap().destination_queues {
                match serde_json::to_string(&temp_val) {
                    Ok(val) => {
                        send_to_topic(&m, &queue, val.as_bytes());
                    }
                    Err(err) => {
                        error!("Failed to serialize the sensor value: {}", err);
                    }
                };
            };

            let temp_humidity = TempHumidityValue {
                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                location: 2,
                temp: temp_f,
                humidity: humidity,
            };

            match serde_json::to_string(&temp_humidity) {
                Ok(val) => {
                    send_to_topic(&m, "/ws/2/grp/temp_humidity", val.as_bytes());
                }
                Err(err) => {
                    error!("Failed to serialize the temp_humidity value: {}", err);
                }
            };



            //////////////////////////
            // CO2 and VOC
            //////////////////////////

            let temp_val = SensorValue {
                id: 52,
                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                value: measurement.co2eq_ppm.to_string(),
            };

            for queue in &sensors.get(&52i16).unwrap().destination_queues {
                match serde_json::to_string(&temp_val) {
                    Ok(val) => {
                        send_to_topic(&m, &queue, val.as_bytes());
                    }
                    Err(err) => {
                        error!("Failed to serialize the sensor value: {}", err);
                    }
                };
            };

            let temp_val = SensorValue {
                id: 53,
                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                value: measurement.tvoc_ppb.to_string(),
            };

            for queue in &sensors.get(&53i16).unwrap().destination_queues {
                match serde_json::to_string(&temp_val) {
                    Ok(val) => {
                        send_to_topic(&m, &queue, val.as_bytes());
                    }
                    Err(err) => {
                        error!("Failed to serialize the sensor value: {}", err);
                    }
                };
            };

            //////////////////////////
            // AIR PRESSURE
            //////////////////////////

            let measurements = bme280.measure().unwrap();
            let mut buf = [b'\0'; 30];
            //Convert to inches of mercury before sending
            let len = dtoa::write(&mut buf[..], measurements.pressure/3386.389).unwrap();
            let flt_as_string = std::str::from_utf8(&buf[..len]).unwrap();

            let temp_val = SensorValue {
                id: 54,
                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                value: String::from(flt_as_string),
            };

            for queue in &sensors.get(&54i16).unwrap().destination_queues {
                match serde_json::to_string(&temp_val) {
                    Ok(val) => {
                        send_to_topic(&m, &queue, val.as_bytes());
                    }
                    Err(err) => {
                        error!("Failed to serialize the sensor value: {}", err);
                    }
                };
            };


            //////////////////////////
            // RADIATION
            //////////////////////////
            let temp_val = SensorValue {
                id: 55,
                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
                value: cpm.to_string(),
            };

            for queue in &sensors.get(&55i16).unwrap().destination_queues {
                match serde_json::to_string(&temp_val) {
                    Ok(val) => {
                        send_to_topic(&m, &queue, val.as_bytes());
                    }
                    Err(err) => {
                        error!("Failed to serialize the sensor value: {}", err);
                    }
                };
            };





            counter = 0;
            info!("Temp: {}, Humidity: {}, Abs Humidity: {}", temp_f, humidity, abs_humidity);
            info!("Temp: {}, Pressure: {}", ((measurements.temperature * 1.8) + 32.0), measurements.pressure/3386.389);
            info!("CO₂eq parts per million: {}", measurement.co2eq_ppm);
            info!("TVOC parts per billion: {}", measurement.tvoc_ppb);
            info!("Radiation CPM: {}", cpm);
        }

        //See above, this timing is important for the SGP30
        thread::sleep(Duration::from_millis(1000));
        counter = counter + 1;
    }
}

fn send_to_topic(m: &Mosquitto, topic: &str, payload: &[u8]) {
    for i in 1..5 {
        match m.publish_wait(topic, payload, 2, false, 1000) {
            Ok(id) => {
                debug!("Message {} published successfully to {} after {} attempts", id, topic, i);
                if i > 1 {
                    info!("Message {} published successfully to {} after {} attempts", id, topic, i);
                }
                break;
            }
            Err(e) => {
                warn!("Failed to enqueue data: {} to topic {}, will retry {} more times", e, topic, 5 - i);
                if i == 5 {
                    error!("Failed to enqueue message after 5 tries to topic {}, it will be dropped", topic);
                }
            }
        };
    }
}

fn setup_logger() -> Result<(), fern::InitError> {
    let base = fern::Dispatch::new();

    let std = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(log::LevelFilter::Info)
//        .level_for("rfm69_to_mq::reliable_datagram", log::LevelFilter::Debug)
        .chain(std::io::stdout())
        .chain(fern::log_file("indoor_sensors.log")?);

    let debug = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(fern::log_file("indoor_sensors_debug.log")?);

    base.chain(std).chain(debug).apply()?;

    Ok(())
}