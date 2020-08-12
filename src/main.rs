use std::error::Error;
use std::process::exit;

#[macro_use]
extern crate envconfig_derive;
#[macro_use]
extern crate log;

use envconfig::Envconfig;


#[derive(Envconfig)]
struct Config {
    #[envconfig(from = "AWAIR_API_KEY")]
    pub api_key: String,

    #[envconfig(from = "AWAIR_DEVICE_TYPE")]
    pub device_type: String,

    #[envconfig(from = "AWAIR_DEVICE_ID")]
    pub device_id: String,
}

fn run(config: Config) {}
fn main() {
    pretty_env_logger::init();

    let result: Result<(), Box<dyn Error>> = Config::init()
        .map(|config| {
            run(config);
        })
        .map_err(|err| Box::new(err) as Box<dyn Error>);

    match result {
        Ok(_) => exit(0),
        Err(err) => {
            error!("{}", err);
            exit(1)
        }
    }
}
