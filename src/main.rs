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
extern crate sensor_lib;

use linux_hal::{I2cdev, Delay};
use htu21d::HTU21D;

use std::thread;
use std::time::Duration;

use std::collections::HashMap;
use std::time::SystemTime;

use sensor_lib::{SensorDefinition, SensorValue, TempHumidityValue, load_from_file};

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

    info!("Connecting to MQ");
    let m = mosquitto_client::Mosquitto::new("indoor_sensors");
    m.connect("localhost", 1883).unwrap();

    loop {
        let temp = (htu21d.read_temperature().unwrap() * 1.8) + 32f32;
        let humidity = htu21d.read_humidity().unwrap();

        let mut buf = [b'\0'; 30];
        let len = dtoa::write(&mut buf[..], temp).unwrap();
        let flt_as_string = std::str::from_utf8(&buf[..len]).unwrap();

        let temp_val = SensorValue{
            id: 50,
            timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
            value: String::from(flt_as_string),
        };

        for queue in &sensors.get(&50i16).unwrap().destination_queues {
            match serde_json::to_string(&temp_val) {
                Ok(val) => {
                    match m.publish_wait(&queue, val.as_bytes(), 2, false, 1000) {
                        Ok(id) => {
                            debug!("Message {} published succesfully", id)
                        }
                        Err(e) => {
                            error!("Failed to enqueue data, message will be dropped: {}", e)
                        }
                    }
                },
                Err(err) => {
                    error!("Failed to serialize the sensor value: {}", err);
                },
            }

        }

        let mut buf = [b'\0'; 30];
        let len = dtoa::write(&mut buf[..], humidity).unwrap();
        let flt_as_string = std::str::from_utf8(&buf[..len]).unwrap();

        let temp_val = SensorValue{
            id: 51,
            timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
            value: String::from(flt_as_string),
        };

        for queue in &sensors.get(&51i16).unwrap().destination_queues {
            match serde_json::to_string(&temp_val) {
                Ok(val) => {
                    match m.publish_wait(&queue, val.as_bytes(), 2, false, 1000) {
                        Ok(id) => {
                            debug!("Message {} published succesfully", id)
                        }
                        Err(e) => {
                            error!("Failed to enqueue data, message will be dropped: {}", e)
                        }
                    }
                },
                Err(err) => {
                    error!("Failed to serialize the sensor value: {}", err);
                },
            }

        }

        let temp_humidity = TempHumidityValue{
            timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
            location: 2,
            temp: temp,
            humidity: humidity,
        };

        match serde_json::to_string(&temp_humidity) {
            Ok(val) => {
                send_to_topic(&m, "/ws/2/grp/temp_humidity", val.as_bytes());
            },
            Err(err) => {
                error!("Failed to serialize the temp_humidity value: {}", err);
            },
        }



        info!("Temp: {}, Humidity: {}", temp, humidity);
        thread::sleep(Duration::from_secs(60));
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