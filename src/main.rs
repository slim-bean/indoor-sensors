#![feature(duration_as_u128)]

#[macro_use]
extern crate log;
extern crate fern;
extern crate chrono;

extern crate embedded_hal as hal;
extern crate linux_embedded_hal as linux_hal;

extern crate mosquitto_client;
extern crate serde_derive;
extern crate serde_yaml;
extern crate serde_json;
extern crate dtoa;

extern crate htu21d;
extern crate sgp30;
extern crate bme280;
extern crate sensor_lib;

extern crate serial;

use std::sync::{Arc,Mutex};
use std::sync::mpsc;

use std::f64::consts::E;

use mosquitto_client::Mosquitto;

mod threads;
use threads::bmp280::Bmp280;
use threads::htu21d::Htu21d;
use threads::sgp30::Sgp30;
use threads::geiger::Geiger;
use threads::sds011::Sds011;


pub struct Payload {
    queue: String,
    bytes: String
}

fn main() {
    setup_logger().unwrap();
    info!("Starting indoor_sensors");


    let (sender, receiver) = mpsc::channel::<Payload>();
    let i2c_mutex = Arc::new(Mutex::new(0i32));

    //FIXME instead of unrwapping all of these we could check for errors so the progam can run missing a sensor
    let bmp280 = Bmp280::new(mpsc::Sender::clone(&sender), Arc::clone(&i2c_mutex)).unwrap();
    let htu21d = Htu21d::new(mpsc::Sender::clone(&sender), Arc::clone(&i2c_mutex)).unwrap();
    let sgp30 = Sgp30::new(mpsc::Sender::clone(&sender), Arc::clone(&i2c_mutex)).unwrap();
    let geiger = Geiger::new(mpsc::Sender::clone(&sender)).unwrap();
    let sds011 = Sds011::new(mpsc::Sender::clone(&sender)).unwrap();

    Bmp280::start_thread(bmp280);
    Htu21d::start_thread(htu21d);
    Sgp30::start_thread(sgp30);
    Geiger::start_thread(geiger);
    Sds011::start_thread(sds011);

    //TODO Send humidity from htu21 over to sgp30
    //TODO Average values in SGP30
    //TODO Save baseline from SGP30
    //TODO Average temp/humidity values?

    info!("Connecting to MQ");
    let m = mosquitto_client::Mosquitto::new("indoor_sensors");
    m.connect("localhost", 1883).unwrap();

    for received in receiver {
        info!("Sending Message to '{}' payload '{}'", received.queue, received.bytes);
        send_to_topic(&m, &received.queue, &received.bytes.as_bytes());
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