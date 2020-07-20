use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use rppal::gpio::Gpio;
use rppal::i2c::I2c;

use as3935::interface::i2c::I2cAddress;
use as3935::{
    Event, HeadOfStormDistance, InterfaceSelection, ListeningParameters, SensorPlacing,
    SignalVerificationThreshold, AS3935,
};

use Payload;

#[derive(Debug)]
pub struct Error {
    message: String,
}

pub struct As3935 {
    sender: Sender<Payload>,
    ls: AS3935,
}

impl As3935 {
    pub fn new(sender: Sender<Payload>, lock: Arc<Mutex<i32>>) -> Result<As3935, Error> {
        info!("Create and Init HTU21Df");

        let gpio = Gpio::new().unwrap();

        let as3935 = AS3935::new(
            InterfaceSelection::I2c(I2c::with_bus(1).unwrap(), I2cAddress::default()),
            gpio.get(23).unwrap().into_input(),
            lock,
        )
        .unwrap();

        Ok(As3935 {
            sender: sender,
            ls: as3935,
        })
    }

    pub fn start_thread(mut sensor: As3935) {
        let events = sensor
            .ls
            .listen(
                ListeningParameters::default()
                    .with_sensor_placing(SensorPlacing::Indoor)
                    .with_signal_verification_threshold(
                        SignalVerificationThreshold::new(2).unwrap(),
                    ),
            )
            .unwrap();

        std::thread::spawn(move || {
            for event in events {
                info!(
                    "{}",
                    match event {
                        Event::Lightning(lightning) => format!(
                            "Lightning detected: {}.",
                            match lightning {
                                HeadOfStormDistance::Kilometers(km) => format!("{} km", km),
                                HeadOfStormDistance::OutOfRange => String::from("out of range"),
                                HeadOfStormDistance::Overhead => String::from("overhead"),
                            }
                        ),
                        Event::Noise => String::from("Noise detected."),
                        Event::Disturbance => String::from("Disturber detected."),
                    }
                )
            }
        });
    }
}
