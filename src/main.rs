
#[macro_use]
extern crate log;
extern crate log4rs;
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

extern crate rppal;
extern crate as3935;

extern crate serial;

extern crate mio_httpc;

use std::sync::{Arc,Mutex};
use std::sync::mpsc;
use std::f32::NAN;
use std::path::Path;

use mosquitto_client::Mosquitto;

mod threads;
use threads::bmp280::Bmp280;
use threads::htu21d::Htu21d;
use threads::sgp30::Sgp30;
use threads::geiger::Geiger;
use threads::sds011::Sds011;
use threads::radiothermostat::RadioThermostat;
use threads::as3935::As3935;


pub struct Payload {
    queue: String,
    bytes: String
}

fn main() {
    //Check if there is a logging config configured in /etc if not just use one local to the app
    let etc_config = Path::new("/etc/indoor_sensors/log4rs.yml");
    let log_config_path = if let true = etc_config.exists() {
        etc_config
    } else {
        Path::new("log4rs.yml")
    };
    log4rs::init_file(log_config_path, Default::default()).expect("Failed to init logger");
    info!("Starting indoor_sensors");


    let (sender, receiver) = mpsc::channel::<Payload>();
    let i2c_mutex = Arc::new(Mutex::new(0i32));

    //FIXME instead of unrwapping all of these we could check for errors so the progam can run missing a sensor
    let bmp280 = Bmp280::new(mpsc::Sender::clone(&sender), Arc::clone(&i2c_mutex)).unwrap();
    let humidity_mutex = Arc::new(Mutex::new((NAN,NAN)));
    let htu21d = Htu21d::new(mpsc::Sender::clone(&sender), Arc::clone(&i2c_mutex), Arc::clone(&humidity_mutex)).unwrap();
    let sgp30 = Sgp30::new(mpsc::Sender::clone(&sender), Arc::clone(&i2c_mutex), Arc::clone(&humidity_mutex)).unwrap();
    let geiger = Geiger::new(mpsc::Sender::clone(&sender)).unwrap();
    let sds011 = Sds011::new(mpsc::Sender::clone(&sender)).unwrap();
    let thermostat = RadioThermostat::new(mpsc::Sender::clone(&sender)).unwrap();
    let lightning_sensor = As3935::new(mpsc::Sender::clone(&sender), Arc::clone(&i2c_mutex)).unwrap();
    

    Bmp280::start_thread(bmp280);
    Htu21d::start_thread(htu21d);
    Sgp30::start_thread(sgp30);
    Geiger::start_thread(geiger);
    Sds011::start_thread(sds011);
    RadioThermostat::start_thread(thermostat);
    As3935::start_thread(lightning_sensor);

    //TODO Average temp/humidity values?

    info!("Connecting to MQ");
    let m = mosquitto_client::Mosquitto::new("indoor_sensors");
    m.connect("localhost", 1883).unwrap();

    for received in receiver {

        if received.queue == "poison" && received.bytes == "poison" {
            error!("One of our threads panicked holding the I2C lock and poisoned it, killing the app");
            panic!("One of our threads panicked holding the I2C lock and poisoned it, killing the app");
        }

        info!("Sending Message to '{}' payload '{}'", received.queue, received.bytes);
        send_to_topic(&m, &received.queue, &received.bytes.as_bytes());
    }
}

fn send_to_topic(m: &Mosquitto, topic: &str, payload: &[u8]) {
    for i in 1..6 {
        match m.publish_wait(topic, payload, 2, false, 1000) {
            Ok(id) => {
                debug!("Message {} published successfully to {} after {} attempts", id, topic, i);
                if i > 1 {
                    info!("Message {} published successfully to {} after {} attempts", id, topic, i);
                }
                break;
            }
            Err(e) => {
                debug!("Failed to enqueue data: {} to topic {}, will retry {} more times", e, topic, 5 - i);
                if i == 5 {
                    error!("Failed to enqueue message after 5 tries to topic {}, it will be dropped", topic);
                    panic!("Failed to enqueue message after 5 tries to topic {}, it will be dropped", topic);
                }
            }
        };
    }
}
