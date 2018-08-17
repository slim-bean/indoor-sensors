
use std::env;

fn main(){
    let target = env::var("TARGET").unwrap();
    if target.contains("armv7") {
        println!("cargo:rustc-link-lib=dylib=mosquitto");
        println!("cargo:rustc-link-lib=dylib=crypto");
        println!("cargo:rustc-link-lib=dylib=ssl");
        println!("cargo:rustc-link-lib=dylib=cares");
        println!("cargo:rustc-link-search=all=mosquitto/usr/lib/arm-linux-gnueabihf");
    }

}


