#!/bin/bash -e

HOST=$1

cargo deb --target=armv7-unknown-linux-gnueabihf
scp target/armv7-unknown-linux-gnueabihf/debian/indoor_sensors_0.1.0_armhf.deb  pi@${HOST}:
ssh pi@${HOST} "sudo dpkg -i indoor_sensors_0.1.0_armhf.deb"